//! The protocol against the robot, not against itself.
//!
//! Every fixture is bytes that existed outside this crate: `*_real.bin` were read off the S1's own
//! UART; `poc_*.bin` are the exact frames that moved its wheels and gimbal on 2026-10-29. A test
//! here failing means the encoder or decoder disagrees with the robot, which is the only
//! disagreement that matters.

use s1_duml::cmd::{self, SystemLed};
use s1_duml::crc::{CRC8_SEED, CRC16_SEED, crc8, crc16};
use s1_duml::dds::{self, PushPeriod};
use s1_duml::events::{ArmorHit, IrHit};
use s1_duml::{Ack, DecodeError, Decoder, Frame};

const HEARTBEAT_REAL: &[u8] = include_bytes!("fixtures/heartbeat_real.bin");
const PUSH_REAL: &[u8] = include_bytes!("fixtures/dds_push_176_real.bin");
const STREAM_REAL: &[u8] = include_bytes!("fixtures/ttyS3_idle_read_stream_4k.bin");

/// The two CRCs the lab checked by hand on this very frame: header `0x75`, trailer bytes `a3 51` on
/// the wire — which, little-endian, is the value `0x51a3`. The lab note wrote the wire order.
#[test]
fn real_heartbeat_crcs() {
    assert_eq!(HEARTBEAT_REAL.len(), 27);
    assert_eq!(crc8(CRC8_SEED, &HEARTBEAT_REAL[0..3]), 0x75);
    assert_eq!(
        crc16(CRC16_SEED, &HEARTBEAT_REAL[0..25]),
        0x51a3,
        "real_heartbeat_crc16"
    );
    assert_eq!(&HEARTBEAT_REAL[25..27], &[0xa3, 0x51]);
}

/// Decoding the robot's frame and encoding it again must give the same bytes: the field order,
/// the length bits and both CRCs are then the robot's, not a belief about them.
#[test]
fn real_heartbeat_round_trips() {
    let frame = Frame::decode(HEARTBEAT_REAL).expect("the robot's own frame decodes");
    assert_eq!(frame.sender, cmd::host::BASE);
    assert_eq!(frame.receiver, cmd::host::CHASSIS);
    assert_eq!((frame.cmd_set, frame.cmd_id), cmd::id::HEARTBEAT);
    assert!(
        frame.ack == Ack::None && !frame.is_ack,
        "the stock heartbeat asks for no ack (attri 0x00)"
    );
    assert_eq!(frame.encode().unwrap(), HEARTBEAT_REAL);
    // And the stock heartbeat builder reproduces it, given only its sequence.
    assert_eq!(
        cmd::heartbeat_stock(frame.seq).encode().unwrap(),
        HEARTBEAT_REAL
    );
}

/// One corrupted byte must be a refused frame, never a frame with a wrong field in it.
#[test]
fn a_flipped_byte_is_refused_by_crc() {
    let mut bad = HEARTBEAT_REAL.to_vec();
    bad[12] ^= 0x01;
    assert!(matches!(
        Frame::decode(&bad),
        Err(DecodeError::BodyCrc { .. })
    ));
    let mut bad_head = HEARTBEAT_REAL.to_vec();
    bad_head[1] ^= 0x01;
    assert!(matches!(
        Frame::decode(&bad_head),
        Err(DecodeError::HeaderCrc { .. })
    ));
}

/// The frames the PoC sent, byte for byte, from the builders with the sequence each file carried.
#[test]
fn poc_frames_are_reproduced_byte_for_byte() {
    assert_eq!(
        cmd::chassis_workmode(0, 1).encode().unwrap(),
        include_bytes!("fixtures/poc_wm.bin")
    );
    assert_eq!(
        cmd::chassis_wheel_speed(0, 40, 40, 40, 40)
            .encode()
            .unwrap(),
        include_bytes!("fixtures/poc_fwd.bin")
    );
    assert_eq!(
        cmd::chassis_wheel_speed(40, 40, -40, -40, 40)
            .encode()
            .unwrap(),
        include_bytes!("fixtures/poc_turn.bin")
    );
    assert_eq!(
        cmd::chassis_wheel_speed(80, 0, 0, 0, 0).encode().unwrap(),
        include_bytes!("fixtures/poc_stop.bin")
    );
    assert_eq!(
        cmd::gimbal_angles(0, 150, 100).encode().unwrap(),
        include_bytes!("fixtures/poc_gim.bin")
    );
    assert_eq!(
        cmd::gimbal_angles(20, 0, 0).encode().unwrap(),
        include_bytes!("fixtures/poc_gimc.bin")
    );
    assert_eq!(
        cmd::heartbeat_proroklab(0).encode().unwrap(),
        include_bytes!("fixtures/poc_hb.bin")
    );
}

/// The sequence is per command: wheel speed and gimbal count independently, from zero.
#[test]
fn sequence_counts_per_command() {
    let mut seq = cmd::SeqCounter::new();
    assert_eq!(seq.next(cmd::id::CHASSIS_WHEEL_SPEED), 0);
    assert_eq!(seq.next(cmd::id::CHASSIS_WHEEL_SPEED), 1);
    assert_eq!(seq.next(cmd::id::GIMBAL_ROTATE), 0);
    assert_eq!(seq.next(cmd::id::HEARTBEAT), 0);
    assert_eq!(seq.next(cmd::id::CHASSIS_WHEEL_SPEED), 2);
}

/// A real telemetry push: 176 bytes, from the motion controller to us, split as the SDK splits it.
#[test]
fn real_dds_push_parses() {
    let frame = Frame::decode(PUSH_REAL).expect("real push decodes");
    assert_eq!(frame.sender, cmd::host::MOTION_CONTROLLER);
    assert_eq!(frame.receiver, cmd::host::BASE);
    assert_eq!((frame.cmd_set, frame.cmd_id), cmd::id::DDS_PUSH);
    let push = PushPeriod::parse(&frame).expect("a push");
    assert_eq!(push.data.len(), 176 - 13 - 2);
    // Only the envelope is asserted here; the subjects of this same push are decoded by the
    // stock's layout in `tests/stock_push.rs`.
    let _ = (push.sub_mode, push.msg_id);
}

/// The decoder over 4 KiB of the UART as it was read — pieces of frames, several frames per read —
/// finds whole frames and nothing else, and resynchronises after garbage.
#[test]
fn decoder_recovers_frames_from_the_raw_read_stream() {
    let mut dec = Decoder::new();
    let mut frames = Vec::new();
    for chunk in STREAM_REAL.chunks(37) {
        frames.extend(dec.push(chunk));
    }
    assert!(frames.len() >= 10, "got {} frames", frames.len());
    assert!(
        frames
            .iter()
            .any(|f| (f.cmd_set, f.cmd_id) == cmd::id::DDS_PUSH && f.wire_len() == 176)
    );
    for f in &frames {
        assert_eq!(Frame::decode(&f.encode().unwrap()).unwrap(), *f);
    }
    // Garbage, then a valid frame: the garbage is counted and the frame comes out whole.
    let mut dec = Decoder::new();
    let mut noisy = vec![0x00, 0x55, 0x01, 0xff];
    noisy.extend_from_slice(HEARTBEAT_REAL);
    let out = dec.push(&noisy);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].encode().unwrap(), HEARTBEAT_REAL);
    assert_eq!(dec.dropped(), 4);
}

///
/// The stock answers every telemetry push with an ack carrying the push's sequence: ours must be
/// its bytes, so the motion controller sees the subscriber it expects.
#[test]
fn push_ack_reproduces_the_stocks_frame_byte_for_byte() {
    const PUSH_ACK_REAL: &[u8] = include_bytes!("fixtures/dds_push_ack_stock_real.bin");
    let real = Frame::decode(PUSH_ACK_REAL).unwrap();
    assert!(real.is_ack);
    assert_eq!(real.ack, Ack::Now);
    assert_eq!((real.cmd_set, real.cmd_id), cmd::id::DDS_PUSH);
    assert_eq!(real.data, vec![0x01]);
    assert_eq!(dds::push_ack(real.seq).encode().unwrap(), PUSH_ACK_REAL);
    assert_eq!(real.seq, 0xc6e5);
}

#[test]
fn add_node_and_add_sub_reproduce_the_stocks_frames_byte_for_byte() {
    // What `dji_hdvt_uav` wrote to the UART on start-up (strace, 2026-10-30): register node 9,
    // then subscribe one subject under msg id 0 at 1 Hz with both flags — attri 0x40 on both.
    const ADD_NODE_REAL: &[u8] = include_bytes!("fixtures/dds_add_node_stock_real.bin");
    const ADD_SUB_REAL: &[u8] = include_bytes!("fixtures/dds_add_sub_stock_real.bin");
    assert_eq!(dds::add_node(0, 9).encode().unwrap(), ADD_NODE_REAL);
    let real = Frame::decode(ADD_SUB_REAL).unwrap();
    assert_eq!(real.ack, Ack::WhenDone);
    assert_eq!(
        dds::add_sub(1, 9, 0, &[0x0002_0003_d7f5_dcfb], 1, true, true)
            .encode()
            .unwrap(),
        ADD_SUB_REAL
    );
}

#[test]
fn add_sub_matches_the_sdk_layout() {
    let f = dds::add_sub(
        0,
        9,
        1,
        &[dds::uid::BATTERY, dds::uid::ATTITUDE],
        20,
        false,
        false,
    );
    assert_eq!((f.cmd_set, f.cmd_id), cmd::id::DDS_ADD_SUB);
    assert_eq!(f.data.len(), 7 + 16);
    assert_eq!(&f.data[0..5], &[9, 1, 0, 0, 2]);
    assert_eq!(&f.data[5..13], &dds::uid::BATTERY.to_le_bytes());
    assert_eq!(&f.data[13..21], &dds::uid::ATTITUDE.to_le_bytes());
    assert_eq!(&f.data[21..23], &20u16.to_le_bytes());
    let d = dds::del_sub(0, 9, 1);
    assert_eq!(d.data, vec![0, 9, 1]);
}

/// Subjects are cut off the front of a push in subscription order, each at its SDK size.
#[test]
fn subjects_parse_in_order_at_their_sizes() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0x10, 0x27]); // adc 10000
    data.extend_from_slice(&25i16.to_le_bytes());
    data.extend_from_slice(&(-1200i32).to_le_bytes());
    data.extend_from_slice(&[87, 0]);
    for v in [90.0f32, -5.0, 1.5] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&[0b0001_0000, 0b0000_0010, 0, 0]);
    let (b, rest) = dds::Battery::parse(&data).unwrap();
    assert_eq!(
        (b.adc_value, b.temperature, b.current, b.percent),
        (10000, 25, -1200, 87)
    );
    let (a, rest) = dds::Attitude::parse(rest).unwrap();
    assert_eq!((a.yaw, a.pitch, a.roll), (90.0, -5.0, 1.5));
    let (s, rest) = dds::SaStatus::parse(rest).unwrap();
    assert!(s.is_pick_up && s.roll_over && !s.static_flag);
    assert!(rest.is_empty());
    assert_eq!(dds::Esc::SIZE, 36);
    assert_eq!(
        dds::Velocity::SIZE + dds::Imu::SIZE + dds::Position::SIZE,
        60
    );
}

/// Armour and infrared events as `protocol.py` unpacks them.
#[test]
fn events_parse_as_the_sdk_unpacks_them() {
    let hit = Frame::request(3, 9, 0, 0x3f, 0x02, vec![0x31, 0x34, 0x12, 0x02, 0x00]);
    let h = ArmorHit::parse(&hit).unwrap();
    assert_eq!((h.index, h.kind, h.mic_value, h.mic_len), (3, 1, 0x1234, 2));
    let ir = Frame::request(3, 9, 0, 0x3f, 0x10, vec![0x25, 1, 7]);
    let i = IrHit::parse(&ir).unwrap();
    assert_eq!(
        (i.role_id, i.skill_id, i.recv_dev, i.recv_ir_pin),
        (2, 5, 1, 7)
    );
    assert!(ArmorHit::parse(&ir).is_none());
}

/// The LED command is what the stock app writes, byte for byte: two frames captured on the UART
/// on 2026-10-30 while the app changed colours (solid teal, then white breathing), reproduced
/// from the struct with the captured sequence numbers.
#[test]
fn system_led_reproduces_the_apps_frames_byte_for_byte() {
    let solid: &[u8] = include_bytes!("fixtures/led_stock_solid_real.bin");
    let breath: &[u8] = include_bytes!("fixtures/led_stock_breath_real.bin");
    let decoded = Frame::decode(solid).expect("a real frame decodes");
    assert_eq!((decoded.cmd_set, decoded.cmd_id), cmd::id::SYSTEM_LED);
    assert_eq!(
        (decoded.sender, decoded.receiver),
        (cmd::host::BASE, cmd::host::LED)
    );
    assert_eq!(
        cmd::system_led(0x001b, &SystemLed::solid(0x00, 0x7f, 0x46))
            .encode()
            .unwrap(),
        solid
    );
    assert_eq!(
        cmd::system_led(0x001c, &SystemLed::breath(255, 255, 255, 500, 500))
            .encode()
            .unwrap(),
        breath
    );
    assert_eq!(cmd::system_led(0, &SystemLed::off()).data[0], 0);
}

/// `chassis_speed` is three little-endian floats, as `drive_speed` packs them.
#[test]
fn chassis_speed_is_three_floats() {
    let f = cmd::chassis_speed(0, 0.5, -0.25, 90.0);
    assert_eq!(f.data.len(), 12);
    assert_eq!(&f.data[0..4], &0.5f32.to_le_bytes());
    assert_eq!(&f.data[8..12], &90.0f32.to_le_bytes());
}

/// A header whose declared length is below the overhead, with a valid header CRC: the public
/// decoder must say so rather than index past the start of the slice.
#[test]
fn a_header_declaring_less_than_the_overhead_is_an_error_not_a_panic() {
    // Six bytes, both CRCs valid: the header CRC over the first three, the frame CRC over the
    // first four — a frame that cannot hold its own eleven-byte header.
    // Padded to thirteen, so the length check at the door is passed and the declared size is
    // what the decoder has to refuse (it used to slice `11..4` here).
    let mut head = [0u8; 13];
    head[..3].copy_from_slice(&[0x55, 6, 0x04]);
    head[3] = crc8(CRC8_SEED, &head[0..3]);
    let body = crc16(CRC16_SEED, &head[0..4]);
    head[4] = (body & 0xff) as u8;
    head[5] = (body >> 8) as u8;
    let err = Frame::decode(&head).expect_err("a 6-byte frame cannot hold its header");
    assert!(err.to_string().contains("declares"), "{err}");
}

/// The stock's muzzle flash, captured on the UART with the app firing (2026-10-30, 15:45:56.117,
/// `capture/hdvt_fire.strace`, 30 identical ones): the launcher-LED command with control 7,
/// effect 3 (flash), green, once, 40 ms on. `cmd::gun_fire_flash` carries that payload; the
/// sender and the ack request are the library's (0xC9, ack when done), not the stock
/// controller's (0x09, none), as every builder here.
#[test]
fn the_muzzle_flash_reproduces_the_stocks_payload() {
    let real: &[u8] = include_bytes!("fixtures/gun_led_fire_stock_real.bin");
    let decoded = Frame::decode(real).expect("a real frame decodes");
    assert_eq!((decoded.cmd_set, decoded.cmd_id), cmd::id::GUN_LED);
    assert_eq!(
        (decoded.sender, decoded.receiver),
        (cmd::host::BASE, cmd::host::GUN)
    );
    assert_eq!(
        decoded.data,
        [0x73, 0x00, 0xff, 0x00, 0x01, 0x28, 0x00, 0x00, 0x00]
    );
    let ours = cmd::gun_fire_flash(0x027a);
    assert_eq!(
        (ours.receiver, ours.cmd_set, ours.cmd_id),
        (cmd::host::GUN, 0x3f, 0x55)
    );
    assert_eq!(
        ours.data, decoded.data,
        "green, flash, 40 ms, once — the stock's bytes"
    );
}
