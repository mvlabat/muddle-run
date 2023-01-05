use crate::{
    framebuffer::FrameNumber,
    messages::{
        DisconnectReason, Message, ReliableClientMessage, ReliableServerMessage,
        UnreliableClientMessage, UnreliableServerMessage,
    },
    wrapped_counter::WrappedCounter,
    TICKS_PER_NETWORK_BROADCAST,
};
use bevy::{ecs::system::Resource, prelude::NonSendMut, utils::Instant};
use bevy_disturbulence::{
    ConnectionChannelsBuilder, MessageChannelMode, MessageChannelSettings, NetworkResource,
    ReliableChannelSettings, UnreliableChannelSettings,
};
use std::{collections::VecDeque, time::Duration};
use thiserror::Error;

pub const CONNECTION_TIMEOUT_MILLIS: u64 = 10000;
const NET_STAT_UPDATE_FACTOR: f32 = 0.2;

pub type MessageId = WrappedCounter<u16>;
pub type SessionId = WrappedCounter<u16>;

#[derive(Debug, Clone, Copy)]
pub enum ConnectionStatus {
    Uninitialized,
    /// Used only on the client side, to mark that the `Initialize` message has
    /// been sent.
    Initialized,
    Connecting,
    Handshaking,
    Connected,
    /// We've received a `Disconnect` event or triggered the process manually.
    /// After we finish the needed clean-up, we switch the status to
    /// `Disconnected`.
    Disconnecting(DisconnectReason),
    Disconnected,
}

#[derive(Debug, Error)]
pub enum AcknowledgeError {
    /// Only actual for acknowledging incoming packets.
    #[error("invalid frame step for an incoming acknowledgment")]
    InvalidStep,
    #[error("acknowledged frame is out of stored range")]
    OutOfRange { start: Option<FrameNumber> },
    #[error("acknowledgement for outgoing packets is inconsistent with the previous one")]
    Inconsistent,
}

#[derive(Debug, Error)]
pub enum AddOutgoingPacketError {
    #[error("adding a new outgoing packet would pop an unacknowledged one")]
    WouldLoseUnacknowledged,
}

// Note: We don't expect clients or server to re-send lost packets. If we detect
// packet loss, we enable redundancy to include the lost updates in future
// packets.
#[derive(Resource)]
pub struct ConnectionState {
    pub handshake_id: MessageId,
    pub session_id: SessionId,
    pub last_valid_message_received_at: Instant,
    status: ConnectionStatus,
    status_updated_at: Instant,
    newest_acknowledged_incoming_packet: Option<FrameNumber>,
    // Packets that are coming to us (not to a peer on the other side of a connection).
    // We acknowledge these packets on receiving an unreliable message and send send the acks
    // later. The least significant bit represents the newest acknowledgement (latest frame).
    incoming_packets_acks: u64,
    // Packets that we send to a peer represented by this connection. Here we store acks sent to us
    // by that peer.
    // The first ack is the oldest one.
    outgoing_packets_acks: VecDeque<Acknowledgment>,
    packet_loss: f32,
    jitter_millis: f32,
    rtt_millis: f32,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            handshake_id: MessageId::new(0),
            session_id: SessionId::new(0),
            last_valid_message_received_at: Instant::now(),
            status: ConnectionStatus::Uninitialized,
            status_updated_at: Instant::now(),
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: u64::MAX - 1,
            outgoing_packets_acks: VecDeque::new(),
            packet_loss: 0.0,
            jitter_millis: 0.0,
            rtt_millis: 100.0,
        }
    }
}

impl ConnectionState {
    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    pub fn status_updated_at(&self) -> Instant {
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
            .take(self.outgoing_packets_to_fill())
            .chain(
                self.outgoing_packets_acks
                    .iter()
                    .map(|ack| ack.is_acknowledged),
            )
            .fold(0, |bitset, ack| bitset << 1 | ack as u64)
    }

    pub fn first_unacknowledged_outgoing_packet(&self) -> Option<FrameNumber> {
        self.outgoing_packets_acks
            .iter()
            .find(|ack| !ack.is_acknowledged)
            .map(|ack| ack.frame_number)
    }

    pub fn set_status(&mut self, status: ConnectionStatus) {
        let session_id = self.session_id;
        let handshake_id = self.handshake_id;

        *self = Self::default();
        self.status = status;
        self.status_updated_at = Instant::now();
        self.session_id = session_id;
        self.handshake_id = handshake_id;
    }

    pub fn add_outgoing_packet(&mut self, frame_number: FrameNumber, sent: Instant) {
        if self.outgoing_packets_acks.len() == 64 {
            self.outgoing_packets_acks.pop_front();
        }
        if let Some(prev_packet) = self.outgoing_packets_acks.back() {
            if prev_packet.frame_number + FrameNumber::new(TICKS_PER_NETWORK_BROADCAST)
                != frame_number
            {
                // This function is expected to receive only a local frame number, which can't
                // have an inconsistent step.
                panic!(
                    "Inconsistent packet step (latest: {}, new: {})",
                    prev_packet.frame_number.value(),
                    frame_number.value()
                );
            }
        }
        self.outgoing_packets_acks.push_back(Acknowledgment {
            frame_number,
            is_acknowledged: false,
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
        // We always store 63 more acks before the newest one.
        let start = newest_acknowledged - FrameNumber::new(TICKS_PER_NETWORK_BROADCAST * 63);
        // We aren't interested in outdated acks, but we are fine to accept acks from
        // far ahead in future, even if it'll make the whole buffer filled with
        // zeroes (except for the newest ack).
        if frame_number < start {
            return Err(AcknowledgeError::OutOfRange { start: Some(start) });
        }

        // Accepts a number of frames, returns how many acks it'll take in the buffer.
        fn bits_for_frame_diff(d: u16) -> Result<u16, AcknowledgeError> {
            if d % TICKS_PER_NETWORK_BROADCAST > 0 {
                return Err(AcknowledgeError::InvalidStep);
            }
            Ok(d / TICKS_PER_NETWORK_BROADCAST)
        }

        let (old_acks_to_drop, shift_new_ack) = match self.newest_acknowledged_incoming_packet {
            // The update is too far ahead, so we need to zero the buffer.
            Some(newest_acknowledged)
                if frame_number > newest_acknowledged + FrameNumber::new(63) =>
            {
                self.incoming_packets_acks = 0;
                (0, 0)
            }
            // The ack is not the newest one, so don't need to shift out any old acks.
            Some(newest_acknowledged) if newest_acknowledged >= frame_number => {
                let d = (newest_acknowledged - frame_number).value();
                let bits = bits_for_frame_diff(d)?;
                (0, bits)
            }
            // The ack is newer than the latest one, so we need to shift out some old ones.
            Some(newest_acknowledged) => {
                let d = (frame_number - newest_acknowledged).value();
                let bits = bits_for_frame_diff(d)?;
                (bits, 0)
            }
            // This is the first ack.
            None => (0, 0),
        };

        self.incoming_packets_acks =
            self.incoming_packets_acks << old_acks_to_drop | 1 << shift_new_ack;
        if newest_acknowledged < frame_number {
            self.newest_acknowledged_incoming_packet = Some(frame_number);
        }

        Ok(())
    }

    /// Applies acknowledgements (sent to us by another peer) of our outgoing
    /// packets.
    pub fn apply_outgoing_acknowledgements(
        &mut self,
        frame_number: FrameNumber,
        mut acknowledgment_bit_set: u64,
    ) -> Result<(), AcknowledgeError> {
        let now = Instant::now();
        // We haven't sent any packets.
        if self.outgoing_packets_acks.is_empty() {
            return Ok(());
        }
        // The least significant bit represents the last frame that a peer acknowledged,
        // so it can't be zero.
        let least_significant_bit_is_set = acknowledgment_bit_set & 1 == 1u64;
        if !least_significant_bit_is_set {
            return Err(AcknowledgeError::Inconsistent);
        }

        let acknowledged_frame_position = self
            .outgoing_packets_acks
            .iter()
            .rev()
            .position(|ack| ack.frame_number == frame_number);
        // A difference between the newest frame that a peer acknowledged and the newest
        // one that we've sent.
        let skip = match acknowledged_frame_position {
            Some(position) => position,
            None => {
                // A client is either lagging behind or is faulty and sends us acks of packets
                // we haven't even sent yet.
                return Err(AcknowledgeError::OutOfRange {
                    start: self
                        .outgoing_packets_acks
                        .front()
                        .map(|ack| ack.frame_number),
                });
            }
        };

        // How many acks are no longer relevant to us (as they are too old).
        let frames_to_forget = skip + self.outgoing_packets_to_fill();
        // How many acks we'll actually write.
        let frames_to_set = self.outgoing_packets_acks.len() - skip;

        acknowledgment_bit_set <<= frames_to_forget;

        self.packet_loss = (acknowledgment_bit_set.count_zeros() as usize - frames_to_forget)
            as f32
            / frames_to_set as f32;

        let newest_acknowledged = self
            .outgoing_packets_acks
            .iter()
            .rfind(|ack| ack.is_acknowledged)
            .map(|ack| ack.frame_number);
        // Tests whether the processed ack is older than the newest one that we
        // received. If it is, we don't need to process it.
        let is_outdated =
            newest_acknowledged.map_or(false, |newest_ack_frame| newest_ack_frame > frame_number);

        if !is_outdated {
            for acknowledgment in self.outgoing_packets_acks.iter_mut().take(frames_to_set) {
                let is_acknowledged = acknowledgment_bit_set >> 63 == 1;
                // Newer acks can't be in conflict with previously sent ones.
                if !is_acknowledged && acknowledgment.is_acknowledged {
                    return Err(AcknowledgeError::Inconsistent);
                }
                if is_acknowledged && !acknowledgment.is_acknowledged {
                    acknowledgment.is_acknowledged = true;
                }
                acknowledgment_bit_set <<= 1;
            }
        }

        assert!(frames_to_set > 0);
        let ack = &mut self.outgoing_packets_acks[frames_to_set - 1];
        assert_eq!(ack.frame_number, frame_number);
        // We want to remember when all acks were received exactly, even if they come
        // unordered.
        ack.acknowledged_at = Some(now);

        self.update_stats(frame_number);

        Ok(())
    }

    fn update_stats(&mut self, frame_number: FrameNumber) {
        // Position of the newest acknowledged frame + 1.
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
            .fold(0u32, |acc, ack| acc + !ack.is_acknowledged as u32);
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
                .as_secs_f32()
                * 1000.0;
            self.rtt_millis += (rtt - self.rtt_millis) * NET_STAT_UPDATE_FACTOR;
        }

        // Calculating mean rtt.
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
        let mean_rtt = acc / count as f32;

        // Calculating jitter.
        let jitter = self
            .outgoing_packets_acks
            .iter()
            .filter_map(|ack| ack.rtt_millis())
            .map(|rtt| (mean_rtt - rtt) * (mean_rtt - rtt))
            .sum::<f32>()
            .sqrt();
        self.jitter_millis += (jitter - self.jitter_millis) * NET_STAT_UPDATE_FACTOR;
    }

    /// How many elements we can still add to the buffer without shifting old
    /// packets out.
    fn outgoing_packets_to_fill(&self) -> usize {
        64 - self.outgoing_packets_acks.len()
    }
}

#[derive(Debug)]
struct Acknowledgment {
    frame_number: FrameNumber,
    is_acknowledged: bool,
    /// This field will be left empty if a package was lost.
    /// Even if we receive acks including this frame later (when it's not the
    /// leading one), `acknowledged_at` will remain being set to `None`.
    acknowledged_at: Option<Instant>,
    sent_at: Instant,
}

impl Acknowledgment {
    pub fn rtt_millis(&self) -> Option<f32> {
        self.acknowledged_at
            .map(|acknowledged_at| (acknowledged_at - self.sent_at).as_secs_f32() * 1000.0)
    }
}

pub fn network_setup(mut net: NonSendMut<NetworkResource>) {
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
    channel_mode: MessageChannelMode::Unreliable {
        settings: UnreliableChannelSettings {
            bandwidth: 4096,
            burst_bandwidth: 1024,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 128,
    packet_buffer_size: 128,
};

const CLIENT_RELIABLE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 1,
    channel_mode: MessageChannelMode::Reliable {
        settings: ReliableChannelSettings {
            bandwidth: 1024 * 1024,
            recv_window_size: 1024,
            send_window_size: 1024,
            burst_bandwidth: 1024,
            init_send: 512,
            resend_time: Duration::from_millis(100),
            initial_rtt: Duration::from_millis(200),
            max_rtt: Duration::from_secs(2),
            rtt_update_factor: 0.1,
            rtt_resend_factor: 1.5,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 1024,
    packet_buffer_size: 1024,
};

const SERVER_RELIABLE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 2,
    channel_mode: MessageChannelMode::Reliable {
        settings: ReliableChannelSettings {
            bandwidth: 1024 * 1024,
            recv_window_size: 1024,
            send_window_size: 1024,
            burst_bandwidth: 1024,
            init_send: 512,
            resend_time: Duration::from_millis(100),
            initial_rtt: Duration::from_millis(200),
            max_rtt: Duration::from_secs(2),
            rtt_update_factor: 0.1,
            rtt_resend_factor: 1.5,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 1024,
    packet_buffer_size: 1024,
};

const SERVER_DELTA_UPDATE_MESSAGE_SETTINGS: MessageChannelSettings = MessageChannelSettings {
    channel: 3,
    channel_mode: MessageChannelMode::Unreliable {
        settings: UnreliableChannelSettings {
            bandwidth: 4096,
            burst_bandwidth: 1024,
        },
        max_message_len: 1024,
    },
    message_buffer_size: 128,
    packet_buffer_size: 128,
};

#[cfg(test)]
mod tests {
    use crate::{
        framebuffer::FrameNumber,
        net::{Acknowledgment, ConnectionState, ConnectionStatus, MessageId, SessionId},
        TICKS_PER_NETWORK_BROADCAST,
    };
    use bevy::utils::Instant;
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
        let now = Instant::now();
        let acknowledgments = acknowledgments
            .iter()
            .enumerate()
            .map(|(i, &acknowledged)| Acknowledgment {
                frame_number: FrameNumber::new(i as u16 * 2),
                is_acknowledged: acknowledged,
                acknowledged_at: if acknowledged { Some(now) } else { None },
                sent_at: now,
            })
            .collect::<Vec<_>>();

        ConnectionState {
            handshake_id: MessageId::new(0),
            session_id: SessionId::new(0),
            last_valid_message_received_at: Instant::now(),
            status: ConnectionStatus::Uninitialized,
            status_updated_at: Instant::now(),
            newest_acknowledged_incoming_packet: None,
            incoming_packets_acks: 0,
            outgoing_packets_acks: VecDeque::from(acknowledgments),
            packet_loss: 0.0,
            jitter_millis: 0.0,
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
        connection_state.add_outgoing_packet(FrameNumber::new(6), Instant::now());
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111111111111111111111111111111111111111111111111111111110010,
        );
        connection_state
            .apply_outgoing_acknowledgements(FrameNumber::new(6), u64::MAX - 4)
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
                FrameNumber::new(30),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b0000000000000001000000000000000000000000000000000000000000000000,
        );
        connection_state
            .apply_outgoing_acknowledgements(
                FrameNumber::new(126),
                0b1111111100000001000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111100000001000000000000000000000000000000000000000000000001,
        );
        connection_state
            .apply_outgoing_acknowledgements(
                FrameNumber::new(126),
                0b1111111111111111000000000000000000000000000000000000000000000001,
            )
            .unwrap();
        assert_eq_bitset!(
            connection_state.outgoing_acknowledgments_bit_set(),
            0b1111111111111111000000000000000000000000000000000000000000000001,
        );
    }
}
