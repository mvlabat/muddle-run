use crate::{
    framebuffer::FrameNumber,
    messages::{
        ReliableClientMessage, ReliableServerMessage, UnreliableClientMessage,
        UnreliableServerMessage,
    },
    TICKS_PER_NETWORK_BROADCAST,
};
use bevy::ecs::ResMut;
use bevy_networking_turbulence::{
    ConnectionChannelsBuilder, MessageChannelMode, MessageChannelSettings, NetworkResource,
    ReliableChannelSettings,
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use thiserror::Error;

const RTT_UPDATE_FACTOR: f32 = 0.2;

#[derive(Debug, Error)]
pub enum AcknowledgeError {
    /// Only actual for acknowledging incoming packets.
    #[error("invalid frame step for an incoming acknowledgment")]
    InvalidStep,
    #[error("acknowledged frame is out of stored range")]
    OutOfRange,
    #[error("acknowledgement for outcoming packets is inconsistent with the previous one")]
    Inconsistent,
}

#[derive(Debug, Error)]
pub enum AddOutcomingPacketError {
    #[error("adding a new outcoming packet would pop an unacknowledged one")]
    WouldLoseUnacknowledged,
}

// Note: We don't expect clients or server to re-send lost packets. If we detect packet loss,
// we enable redundancy to include the lost updates in future packets.
pub struct ConnectionState {
    newest_acknowledged_incoming_packet: Option<FrameNumber>,
    incoming_packets_acks: u64,
    outcoming_packets_acks: VecDeque<Acknowledgment>,
    packet_loss: f32,
    jitter: f32,
    rtt_millis: f32,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: u64::MAX - 1,
            outcoming_packets_acks: VecDeque::new(),
            packet_loss: 0.0,
            jitter: 0.0,
            rtt_millis: 100.0,
        }
    }
}

impl ConnectionState {
    pub fn packet_loss(&self) -> f32 {
        self.packet_loss
    }

    pub fn jitter(&self) -> f32 {
        self.jitter
    }

    pub fn rtt_millis(&self) -> f32 {
        self.rtt_millis
    }

    pub fn incoming_acknowledgments(&self) -> (Option<FrameNumber>, u64) {
        (
            self.newest_acknowledged_incoming_packet,
            self.incoming_packets_acks,
        )
    }

    pub fn outcoming_acknowledgments_bit_set(&self) -> u64 {
        std::iter::repeat(true)
            .take(self.frames_to_fill())
            .chain(
                self.outcoming_packets_acks
                    .iter()
                    .map(|ack| ack.acknowledged.is_some()),
            )
            .fold(0, |bitset, ack| bitset << 1 | ack as u64)
    }

    pub fn first_unacknowledged_outcoming_packet(&self) -> Option<FrameNumber> {
        self.outcoming_packets_acks
            .iter()
            .find(|ack| ack.acknowledged.is_none())
            .map(|ack| ack.frame_number)
    }

    pub fn add_outcoming_packet(
        &mut self,
        frame_number: FrameNumber,
        sent: Instant,
    ) -> Result<(), AddOutcomingPacketError> {
        if self.outcoming_packets_acks.len() == 64 {
            let oldest_packet = self.outcoming_packets_acks.front().unwrap();
            if oldest_packet.acknowledged.is_none() {
                return Err(AddOutcomingPacketError::WouldLoseUnacknowledged);
            }
            self.outcoming_packets_acks.pop_front();
        }
        if let Some(prev_packet) = self.outcoming_packets_acks.back() {
            if prev_packet.frame_number + FrameNumber::new(TICKS_PER_NETWORK_BROADCAST)
                != frame_number
            {
                panic!(
                    "Inconsistent packet step (latest: {}, new: {})",
                    prev_packet.frame_number.value(),
                    frame_number.value()
                );
            }
        }
        self.outcoming_packets_acks.push_back(Acknowledgment {
            frame_number,
            acknowledged: None,
            sent,
        });
        Ok(())
    }

    pub fn acknowledge_incoming(
        &mut self,
        frame_number: FrameNumber,
    ) -> Result<(), AcknowledgeError> {
        let newest_acknowledged = self
            .newest_acknowledged_incoming_packet
            .unwrap_or_else(|| frame_number - FrameNumber::new(TICKS_PER_NETWORK_BROADCAST));
        let can_acknowledge_frame = (newest_acknowledged
            - FrameNumber::new(TICKS_PER_NETWORK_BROADCAST * 63)
            ..=newest_acknowledged + FrameNumber::new(TICKS_PER_NETWORK_BROADCAST * 64))
            .contains(&frame_number);
        if !can_acknowledge_frame {
            return Err(AcknowledgeError::OutOfRange);
        }

        fn bits_for_frame_diff(d: u16) -> Result<u16, AcknowledgeError> {
            if d % TICKS_PER_NETWORK_BROADCAST > 0 {
                return Err(AcknowledgeError::InvalidStep);
            }
            Ok(d / TICKS_PER_NETWORK_BROADCAST)
        }
        let (shift_lhs, shift_rhs) = match self.newest_acknowledged_incoming_packet {
            Some(newest_acknowledged) if newest_acknowledged >= frame_number => {
                let d = (newest_acknowledged - frame_number).value();
                let bits = bits_for_frame_diff(d)?;
                (0, bits)
            }
            Some(newest_acknowledged) => {
                let d = (frame_number - newest_acknowledged).value();
                let bits = bits_for_frame_diff(d)?;
                (bits, 0)
            }
            None => (0, 0),
        };

        self.incoming_packets_acks = self.incoming_packets_acks << shift_lhs | 1 << shift_rhs;
        if newest_acknowledged < frame_number {
            self.newest_acknowledged_incoming_packet = Some(frame_number);
        }

        Ok(())
    }

    pub fn apply_outcoming_acknowledgements(
        &mut self,
        frame_number: FrameNumber,
        mut acknowledgment_bit_set: u64,
    ) -> Result<(), AcknowledgeError> {
        let now = Instant::now();
        if self.outcoming_packets_acks.is_empty() {
            return Ok(());
        }
        let last_bit_is_set = acknowledgment_bit_set & 1 == 1u64;
        if !last_bit_is_set {
            return Err(AcknowledgeError::Inconsistent);
        }

        let requested_frame_position = self
            .outcoming_packets_acks
            .iter()
            .rev()
            .position(|ack| ack.frame_number == frame_number);
        let skip = match requested_frame_position {
            Some(position) => position,
            None => return Err(AcknowledgeError::OutOfRange),
        };

        let frames_to_forget = skip + self.frames_to_fill();
        let frames_to_set = self.outcoming_packets_acks.len() - skip;

        acknowledgment_bit_set <<= frames_to_forget;

        self.packet_loss = (acknowledgment_bit_set.count_zeros() as usize - frames_to_forget)
            as f32
            / frames_to_set as f32;

        for acknowledgment in self.outcoming_packets_acks.iter_mut().take(frames_to_set) {
            let acknowledged = acknowledgment_bit_set >> 63 != 0;
            if !acknowledged && acknowledgment.acknowledged.is_some() {
                return Err(AcknowledgeError::Inconsistent);
            }
            if acknowledged && acknowledgment.acknowledged.is_none() {
                acknowledgment.acknowledged = Some(now);
            }
            acknowledgment_bit_set <<= 1;
        }
        // let ack = &mut self.outcoming_packets_acks[frames_to_set];
        // if ack.acknowledged.is_none() {
        //     assert_eq!(ack.frame_number, frame_number);
        //     ack.acknowledged = Some(now);
        // }

        self.update_stats(frame_number);

        Ok(())
    }

    fn update_stats(&mut self, frame_number: FrameNumber) {
        // Position of the acknowledged frame + 1 (basically the length, but it should also take into account unordered updates).
        let expected_acknowledged_count = self
            .outcoming_packets_acks
            .iter()
            .position(|ack| ack.frame_number == frame_number)
            .map(|pos| pos + 1)
            .unwrap_or(0);

        let outcoming_unacknowledged_count = self
            .outcoming_packets_acks
            .iter()
            .take(expected_acknowledged_count)
            .fold(0u32, |acc, ack| acc + ack.acknowledged.is_none() as u32);
        let incoming_unacknowledged_count = self.incoming_packets_acks.count_zeros();
        self.packet_loss = (outcoming_unacknowledged_count + incoming_unacknowledged_count) as f32
            / (expected_acknowledged_count + 64) as f32;

        if expected_acknowledged_count > 0 {
            let acknowledged_frame = self
                .outcoming_packets_acks
                .get(expected_acknowledged_count - 1)
                .unwrap();
            // TODO: fix this somehow to be callable on acknowledging incoming packets?
            let rtt = (acknowledged_frame
                .acknowledged
                .expect("Expected the currently acknowledged frame to have a timestamp")
                - acknowledged_frame.sent)
                .as_secs_f32()
                * 1000.0;
            self.rtt_millis += (rtt - self.rtt_millis) * RTT_UPDATE_FACTOR;
        }
    }

    fn frames_to_fill(&self) -> usize {
        64 - self.outcoming_packets_acks.len()
    }
}

struct Acknowledgment {
    frame_number: FrameNumber,
    acknowledged: Option<Instant>,
    sent: Instant,
}

pub fn network_setup(mut net: ResMut<NetworkResource>) {
    net.set_channels_builder(|builder: &mut ConnectionChannelsBuilder| {
        builder
            .register::<UnreliableClientMessage>(CLIENT_INPUT_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<ReliableClientMessage>(CLIENT_RELIABLE_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<ReliableServerMessage>(SERVER_RELIABLE_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<UnreliableServerMessage>(SERVER_DELTA_UPDATE_MESSAGE_SETTINGS)
            .unwrap();
    });
}

const CLIENT_INPUT_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 0,
    channel_mode: MessageChannelMode::Reliable {
        reliability_settings: ReliableChannelSettings {
            bandwidth: 4096,
            recv_window_size: 1024,
            send_window_size: 1024,
            burst_bandwidth: 1024,
            init_send: 512,
            wakeup_time: Duration::from_millis(100),
            initial_rtt: Duration::from_millis(200),
            max_rtt: Duration::from_secs(2),
            rtt_update_factor: 0.1,
            rtt_resend_factor: 1.5,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 8,
    packet_buffer_size: 8,
};

const CLIENT_RELIABLE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 1,
    channel_mode: MessageChannelMode::Reliable {
        reliability_settings: ReliableChannelSettings {
            bandwidth: 4096,
            recv_window_size: 1024,
            send_window_size: 1024,
            burst_bandwidth: 1024,
            init_send: 512,
            wakeup_time: Duration::from_millis(100),
            initial_rtt: Duration::from_millis(200),
            max_rtt: Duration::from_secs(2),
            rtt_update_factor: 0.1,
            rtt_resend_factor: 1.5,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 8,
    packet_buffer_size: 8,
};

const SERVER_RELIABLE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 2,
    channel_mode: MessageChannelMode::Reliable {
        reliability_settings: ReliableChannelSettings {
            bandwidth: 4096,
            recv_window_size: 1024,
            send_window_size: 1024,
            burst_bandwidth: 1024,
            init_send: 512,
            wakeup_time: Duration::from_millis(100),
            initial_rtt: Duration::from_millis(200),
            max_rtt: Duration::from_secs(2),
            rtt_update_factor: 0.1,
            rtt_resend_factor: 1.5,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 8,
    packet_buffer_size: 8,
};

const SERVER_DELTA_UPDATE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 3,
    channel_mode: MessageChannelMode::Unreliable,
    message_buffer_size: 8,
    packet_buffer_size: 8,
};

#[cfg(test)]
mod tests {
    use crate::{
        framebuffer::FrameNumber,
        net::{Acknowledgment, ConnectionState},
        TICKS_PER_NETWORK_BROADCAST,
    };
    use std::{collections::VecDeque, time::Instant};

    macro_rules! assert_eq_bitset {
        ($left:expr, $right:expr $(,)?) => {{
            if $left != $right {
                panic!(
                    r#"assertion failed: `(left == right)`
  left: `0b{:064b}`,
 right: `0b{:064b}`"#,
                    $left, $right as u64
                );
            }
        }};
    }

    fn init_connection_state(acknowledgments: Option<Vec<bool>>) -> ConnectionState {
        let acknowledgments = acknowledgments.unwrap_or_else(|| vec![true; 64]);
        assert!(acknowledgments.len() <= 64);
        let now = Instant::now();
        let acknowledgments = acknowledgments
            .iter()
            .enumerate()
            .map(|(i, &acknowledged)| Acknowledgment {
                frame_number: FrameNumber::new(i as u16),
                acknowledged: if acknowledged { Some(now) } else { None },
                sent: now,
            })
            .collect::<Vec<_>>();

        ConnectionState {
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: 0,
            outcoming_packets_acks: VecDeque::from(acknowledgments),
            packet_loss: 0.0,
            jitter: 0.0,
            rtt_millis: 0.0,
        }
    }

    #[test]
    fn test_incoming_acknowledgment() {
        let mut connection_state = ConnectionState::default();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(frame_number, None);
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111110
        );

        connection_state
            .acknowledge_incoming(FrameNumber::new(0))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(frame_number, Some(FrameNumber::new(0)));
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111111
        );

        connection_state
            .acknowledge_incoming(FrameNumber::new(3 * TICKS_PER_NETWORK_BROADCAST))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(
            frame_number,
            Some(FrameNumber::new(3 * TICKS_PER_NETWORK_BROADCAST))
        );
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111001
        );

        connection_state
            .acknowledge_incoming(FrameNumber::new(1 * TICKS_PER_NETWORK_BROADCAST))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(
            frame_number,
            Some(FrameNumber::new(3 * TICKS_PER_NETWORK_BROADCAST))
        );
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111101
        );

        // Asserts idempotency.
        connection_state
            .acknowledge_incoming(FrameNumber::new(1 * TICKS_PER_NETWORK_BROADCAST))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(
            frame_number,
            Some(FrameNumber::new(3 * TICKS_PER_NETWORK_BROADCAST))
        );
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111101
        );
    }

    #[test]
    fn test_outcoming_acknowledgment() {
        let mut connection_state = init_connection_state(Some(vec![false, false, true]));
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b1111111111111111111111111111111111111111111111111111111111111001,
        );
        connection_state
            .apply_outcoming_acknowledgements(FrameNumber::new(1), u64::MAX - 2)
            .unwrap();
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b1111111111111111111111111111111111111111111111111111111111111011,
        );

        let mut connection_state = init_connection_state(Some(vec![false; 64]));
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b0000000000000000000000000000000000000000000000000000000000000000,
        );
        connection_state
            .apply_outcoming_acknowledgements(
                FrameNumber::new(15),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b0000000000000001000000000000000000000000000000000000000000000000,
        );
        connection_state
            .apply_outcoming_acknowledgements(
                FrameNumber::new(63),
                0b1111111100000001000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b1111111100000001000000000000000000000000000000000000000000000001,
        );
        connection_state
            .apply_outcoming_acknowledgements(
                FrameNumber::new(63),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outcoming_acknowledgments_bit_set(),
            0b1111111111111111000000000000000000000000000000000000000000000001,
        );
    }
}
