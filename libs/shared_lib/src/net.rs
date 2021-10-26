use crate::{
    framebuffer::FrameNumber,
    messages::{
        Message, ReliableClientMessage, ReliableServerMessage, UnreliableClientMessage,
        UnreliableServerMessage,
    },
    wrapped_counter::WrappedCounter,
    TICKS_PER_NETWORK_BROADCAST,
};
use bevy::ecs::system::ResMut;
use bevy_networking_turbulence::{
    ConnectionChannelsBuilder, MessageChannelMode, MessageChannelSettings, NetworkResource,
    ReliableChannelSettings,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::VecDeque;
use thiserror::Error;

pub const CONNECTION_TIMEOUT_MILLIS: u64 = 2000;
const RTT_UPDATE_FACTOR: f32 = 0.2;
const JITTER_DECREASE_THRESHOLD_SECS: u64 = 1;

pub type MessageId = WrappedCounter<u16>;
pub type SessionId = WrappedCounter<u16>;

#[derive(Debug, Clone, Copy)]
pub enum ConnectionStatus {
    Uninitialized,
    /// Used only on the client side, to mark that the `Initialize` message has been sent.
    Initialized,
    Connecting,
    Handshaking,
    Connected,
    /// We've received a `Disconnect` event or triggered the process manually. After we finish
    /// the needed clean-up, we switch the status to `Disconnected`.
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Error)]
pub enum AcknowledgeError {
    /// Only actual for acknowledging incoming packets.
    #[error("invalid frame step for an incoming acknowledgment")]
    InvalidStep,
    #[error("acknowledged frame is out of stored range")]
    OutOfRange {
        start: Option<FrameNumber>,
        end: Option<FrameNumber>,
    },
    #[error("acknowledgement for outgoing packets is inconsistent with the previous one")]
    Inconsistent,
}

#[derive(Debug, Error)]
pub enum AddOutgoingPacketError {
    #[error("adding a new outgoing packet would pop an unacknowledged one")]
    WouldLoseUnacknowledged,
}

// Note: We don't expect clients or server to re-send lost packets. If we detect packet loss,
// we enable redundancy to include the lost updates in future packets.
pub struct ConnectionState {
    pub handshake_id: MessageId,
    pub session_id: SessionId,
    pub last_message_received_at: DateTime<Utc>,
    status: ConnectionStatus,
    status_updated_at: DateTime<Utc>,
    newest_acknowledged_incoming_packet: Option<FrameNumber>,
    // Packets that are incoming to us (not to a peer on the other side of a connection).
    // We acknowledge these packets on receiving an unreliable message and send send the acks later.
    incoming_packets_acks: u64,
    // Packets that we send to a peer represented by this connection.
    // Here we store acks sent to us by that peer.
    outgoing_packets_acks: VecDeque<Acknowledgment>,
    packet_loss: f32,
    jitter_millis: f32,
    last_increased_jitter: DateTime<Utc>,
    rtt_millis: f32,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            handshake_id: MessageId::new(0),
            session_id: SessionId::new(0),
            last_message_received_at: Utc::now(),
            status: ConnectionStatus::Uninitialized,
            status_updated_at: Utc::now(),
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: u64::MAX - 1,
            outgoing_packets_acks: VecDeque::new(),
            packet_loss: 0.0,
            jitter_millis: 0.0,
            last_increased_jitter: Utc::now()
                - Duration::from_std(std::time::Duration::from_secs(
                    JITTER_DECREASE_THRESHOLD_SECS,
                ))
                .unwrap(),
            rtt_millis: 100.0,
        }
    }
}

impl ConnectionState {
    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    pub fn status_updated_at(&self) -> DateTime<Utc> {
        self.status_updated_at
    }

    pub fn set_initial_rtt_millis(&mut self, rtt_millis: f32) {
        self.rtt_millis = rtt_millis;
    }

    pub fn packet_loss(&self) -> f32 {
        self.packet_loss
    }

    pub fn jitter_millis(&self) -> f32 {
        self.jitter_millis
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

    pub fn outgoing_acknowledgments_bit_set(&self) -> u64 {
        std::iter::repeat(true)
            .take(self.frames_to_fill())
            .chain(
                self.outgoing_packets_acks
                    .iter()
                    .map(|ack| ack.acknowledged),
            )
            .fold(0, |bitset, ack| bitset << 1 | ack as u64)
    }

    pub fn first_unacknowledged_outgoing_packet(&self) -> Option<FrameNumber> {
        self.outgoing_packets_acks
            .iter()
            .find(|ack| !ack.acknowledged)
            .map(|ack| ack.frame_number)
    }

    pub fn set_status(&mut self, status: ConnectionStatus) {
        let session_id = self.session_id;
        let handshake_id = self.handshake_id;

        *self = Self::default();
        self.status = status;
        self.status_updated_at = Utc::now();
        self.session_id = session_id;
        self.handshake_id = handshake_id;
    }

    pub fn add_outgoing_packet(&mut self, frame_number: FrameNumber, sent: DateTime<Utc>) {
        if self.outgoing_packets_acks.len() == 64 {
            self.outgoing_packets_acks.pop_front();
        }
        if let Some(prev_packet) = self.outgoing_packets_acks.back() {
            if prev_packet.frame_number + FrameNumber::new(TICKS_PER_NETWORK_BROADCAST)
                != frame_number
            {
                // TODO: don't panic. Clients might be able to DoS?
                panic!(
                    "Inconsistent packet step (latest: {}, new: {})",
                    prev_packet.frame_number.value(),
                    frame_number.value()
                );
            }
        }
        self.outgoing_packets_acks.push_back(Acknowledgment {
            frame_number,
            acknowledged: false,
            acknowledged_at: None,
            sent_at: sent,
        });
    }

    pub fn acknowledge_incoming(
        &mut self,
        frame_number: FrameNumber,
    ) -> Result<(), AcknowledgeError> {
        let newest_acknowledged = self
            .newest_acknowledged_incoming_packet
            .unwrap_or_else(|| frame_number - FrameNumber::new(TICKS_PER_NETWORK_BROADCAST));
        let start = newest_acknowledged - FrameNumber::new(TICKS_PER_NETWORK_BROADCAST * 63);
        let end = newest_acknowledged + FrameNumber::new(TICKS_PER_NETWORK_BROADCAST * 64);
        let can_acknowledge_frame = (start..=end).contains(&frame_number);
        if !can_acknowledge_frame {
            return Err(AcknowledgeError::OutOfRange {
                start: Some(start),
                end: Some(end),
            });
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

        // TODO: debug the rare `attempt to shift left with overflow` panic.
        self.incoming_packets_acks = self.incoming_packets_acks << shift_lhs | 1 << shift_rhs;
        if newest_acknowledged < frame_number {
            self.newest_acknowledged_incoming_packet = Some(frame_number);
        }

        Ok(())
    }

    pub fn apply_outgoing_acknowledgements(
        &mut self,
        frame_number: FrameNumber,
        mut acknowledgment_bit_set: u64,
    ) -> Result<(), AcknowledgeError> {
        let now = Utc::now();
        if self.outgoing_packets_acks.is_empty() {
            return Ok(());
        }
        let last_bit_is_set = acknowledgment_bit_set & 1 == 1u64;
        if !last_bit_is_set {
            return Err(AcknowledgeError::Inconsistent);
        }

        let requested_frame_position = self
            .outgoing_packets_acks
            .iter()
            .rev()
            .position(|ack| ack.frame_number == frame_number);
        let skip = match requested_frame_position {
            Some(position) => position,
            None => {
                return Err(AcknowledgeError::OutOfRange {
                    start: self
                        .outgoing_packets_acks
                        .front()
                        .map(|ack| ack.frame_number),
                    end: self
                        .outgoing_packets_acks
                        .back()
                        .map(|ack| ack.frame_number),
                })
            }
        };

        let frames_to_forget = skip + self.frames_to_fill();
        let frames_to_set = self.outgoing_packets_acks.len() - skip;

        acknowledgment_bit_set <<= frames_to_forget;

        self.packet_loss = (acknowledgment_bit_set.count_zeros() as usize - frames_to_forget)
            as f32
            / frames_to_set as f32;

        let last_acknowledged = self
            .outgoing_packets_acks
            .iter()
            .rfind(|ack| ack.acknowledged)
            .map(|ack| ack.frame_number);
        for acknowledgment in self.outgoing_packets_acks.iter_mut().take(frames_to_set) {
            let acknowledged = acknowledgment_bit_set >> 63 != 0;
            let is_not_outdated =
                last_acknowledged.map_or(true, |last_ack_frame| frame_number > last_ack_frame);
            if !acknowledged && acknowledgment.acknowledged && is_not_outdated {
                return Err(AcknowledgeError::Inconsistent);
            }
            if acknowledged && !acknowledgment.acknowledged {
                acknowledgment.acknowledged = true;
            }
            acknowledgment_bit_set <<= 1;
        }

        assert!(frames_to_set > 0);
        let ack = &mut self.outgoing_packets_acks[frames_to_set - 1];
        assert_eq!(ack.frame_number, frame_number);
        ack.acknowledged_at = Some(now);

        self.update_stats(frame_number);

        Ok(())
    }

    fn update_stats(&mut self, frame_number: FrameNumber) {
        // Position of the acknowledged frame + 1 (basically the length, but it should also take into account unordered updates).
        let expected_acknowledged_count = self
            .outgoing_packets_acks
            .iter()
            .position(|ack| ack.frame_number == frame_number)
            .map(|pos| pos + 1)
            .unwrap_or(0);

        // Calculating packet loss.
        let outgoing_unacknowledged_count = self
            .outgoing_packets_acks
            .iter()
            .take(expected_acknowledged_count)
            .fold(0u32, |acc, ack| acc + !ack.acknowledged as u32);
        let incoming_unacknowledged_count = self.incoming_packets_acks.count_zeros();
        self.packet_loss = (outgoing_unacknowledged_count + incoming_unacknowledged_count) as f32
            / (expected_acknowledged_count + 64) as f32;

        // Calculating rtt.
        if expected_acknowledged_count > 0 {
            let acknowledged_frame = self
                .outgoing_packets_acks
                .get(expected_acknowledged_count - 1)
                .unwrap();
            // TODO: fix this somehow to be callable on acknowledging incoming packets?
            let rtt = (acknowledged_frame
                .acknowledged_at
                .expect("Expected the currently acknowledged frame to have a timestamp")
                - acknowledged_frame.sent_at)
                .to_std()
                .unwrap()
                .as_secs_f32()
                * 1000.0;
            self.rtt_millis += (rtt - self.rtt_millis) * RTT_UPDATE_FACTOR;
        }

        // Calculating average rtt.
        let mut acc = 0.0;
        let mut count = 0;
        for rtt in self
            .outgoing_packets_acks
            .iter()
            .filter_map(|ack| ack.rtt_millis())
        {
            acc += rtt;
            count += 1;
        }
        let avg_rtt = acc / count as f32;

        // Calculating jitter.
        let mut jitter = 0.0f32;
        for rtt in self
            .outgoing_packets_acks
            .iter()
            .filter_map(|ack| ack.rtt_millis())
        {
            jitter = jitter.max((avg_rtt - rtt).abs() * 1.1);
        }
        if jitter > self.jitter_millis {
            self.last_increased_jitter = Utc::now();
            self.jitter_millis = jitter;
        } else if Utc::now()
            .signed_duration_since(self.last_increased_jitter)
            .to_std()
            .unwrap()
            > std::time::Duration::from_secs(JITTER_DECREASE_THRESHOLD_SECS)
        {
            self.jitter_millis = self.jitter_millis + (jitter - self.jitter_millis) * 0.1;
        }
    }

    fn frames_to_fill(&self) -> usize {
        64 - self.outgoing_packets_acks.len()
    }
}

#[derive(Debug)]
struct Acknowledgment {
    frame_number: FrameNumber,
    acknowledged: bool,
    /// This field will be left empty if a package was lost.
    /// Even if we receive related updates later, `acknowledged_at` will remain being set to `None`.
    acknowledged_at: Option<DateTime<Utc>>,
    sent_at: DateTime<Utc>,
}

impl Acknowledgment {
    pub fn rtt_millis(&self) -> Option<f32> {
        self.acknowledged_at.map(|acknowledged_at| {
            (acknowledged_at - self.sent_at)
                .to_std()
                .unwrap()
                .as_secs_f32()
                * 1000.0
        })
    }
}

pub fn network_setup(mut net: ResMut<NetworkResource>) {
    net.set_channels_builder(|builder: &mut ConnectionChannelsBuilder| {
        builder
            .register::<Message<UnreliableClientMessage>>(CLIENT_INPUT_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<Message<ReliableClientMessage>>(CLIENT_RELIABLE_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<Message<ReliableServerMessage>>(SERVER_RELIABLE_MESSAGE_SETTINGS)
            .unwrap();
        builder
            .register::<Message<UnreliableServerMessage>>(SERVER_DELTA_UPDATE_MESSAGE_SETTINGS)
            .unwrap();
    });
}

const CLIENT_INPUT_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 0,
    channel_mode: MessageChannelMode::Unreliable,
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
            wakeup_time: std::time::Duration::from_millis(100),
            initial_rtt: std::time::Duration::from_millis(200),
            max_rtt: std::time::Duration::from_secs(2),
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
            wakeup_time: std::time::Duration::from_millis(100),
            initial_rtt: std::time::Duration::from_millis(200),
            max_rtt: std::time::Duration::from_secs(2),
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
        net::{Acknowledgment, ConnectionState, ConnectionStatus, MessageId, SessionId},
        TICKS_PER_NETWORK_BROADCAST,
    };
    use chrono::Utc;
    use std::collections::VecDeque;

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
        let now = Utc::now();
        let acknowledgments = acknowledgments
            .iter()
            .enumerate()
            .map(|(i, &acknowledged)| Acknowledgment {
                frame_number: FrameNumber::new(i as u16),
                acknowledged,
                acknowledged_at: if acknowledged { Some(now) } else { None },
                sent_at: now,
            })
            .collect::<Vec<_>>();

        ConnectionState {
            handshake_id: MessageId::new(0),
            session_id: SessionId::new(0),
            last_message_received_at: Utc::now(),
            status: ConnectionStatus::Uninitialized,
            status_updated_at: Utc::now(),
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: 0,
            outgoing_packets_acks: VecDeque::from(acknowledgments),
            packet_loss: 0.0,
            jitter_millis: 0.0,
            last_increased_jitter: Utc::now(),
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
    fn test_incoming_acknowledgment_with_overflow() {
        let mut connection_state = ConnectionState::default();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(frame_number, None);
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111110
        );

        connection_state
            .acknowledge_incoming(FrameNumber::new(u16::MAX - 1))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(frame_number, Some(FrameNumber::new(u16::MAX - 1)));
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111111
        );

        connection_state
            .acknowledge_incoming(FrameNumber::new(u16::MAX - 1) + FrameNumber::new(2))
            .unwrap();
        let (frame_number, acks) = connection_state.incoming_acknowledgments();
        assert_eq!(frame_number, Some(FrameNumber::new(0)));
        assert_eq_bitset!(
            acks,
            0b1111111111111111111111111111111111111111111111111111111111111111
        );
    }

    #[test]
    fn test_outgoing_acknowledgment() {
        let mut connection_state = init_connection_state(Some(vec![false, false, true]));
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111111111111111111111111111111111111111111111111111111111001,
        );
        connection_state
            .apply_outgoing_acknowledgements(FrameNumber::new(1), u64::MAX - 2)
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111111111111111111111111111111111111111111111111111111111011,
        );

        let mut connection_state = init_connection_state(Some(vec![false; 64]));
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b0000000000000000000000000000000000000000000000000000000000000000,
        );
        connection_state
            .apply_outgoing_acknowledgements(
                FrameNumber::new(15),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b0000000000000001000000000000000000000000000000000000000000000000,
        );
        connection_state
            .apply_outgoing_acknowledgements(
                FrameNumber::new(63),
                0b1111111100000001000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111100000001000000000000000000000000000000000000000000000001,
        );
        connection_state
            .apply_outgoing_acknowledgements(
                FrameNumber::new(63),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111111111111000000000000000000000000000000000000000000000001,
        );
    }
}
