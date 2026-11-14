use serde_json::{Value, json};
use voloo::{
    firmware::{AbStatus, Image},
    protocol::{self, FrameDecoder, Matched, Uid},
};

fn bundle(version: u16) -> Value {
    let a = app(b'A');
    let b = app(b'B');
    let mut factory = vec![255; 58 * 1024];
    let tag = b"VolooSwitch:X035:AB:BOOT:1\0";
    factory[192..192 + tag.len()].copy_from_slice(tag);
    factory[0x2000..0x2000 + a.len()].copy_from_slice(&a);
    factory[0x8000..0x8000 + b.len()].copy_from_slice(&b);
    json!({"format":"voloo-x035-ab-v1","hardware_version":0x300,"firmware_version":version,"slot_a":a,"slot_b":b,"factory":factory})
}
fn app(slot: u8) -> Vec<u8> {
    let mut app = vec![0u8; 512];
    let mut tag = b"VolooSwitch:X035:AB:1:".to_vec();
    tag.push(slot);
    tag.push(0);
    app[176..176 + tag.len()].copy_from_slice(&tag);
    app
}
#[test]
fn packages_require_matching_target_slots_and_configuration_safe_factory() {
    let b = bundle(0x20a);
    assert!(matches!(
        Image::parse(&serde_json::to_vec(&b).unwrap()).unwrap(),
        Image::Ab(_)
    ));
    for field in ["format", "hardware_version", "slot_b", "factory"] {
        let mut bad = b.clone();
        bad[field] = match field {
            "format" => json!("another board"),
            "hardware_version" => json!(0x400),
            "slot_b" => bad["slot_a"].clone(),
            _ => json!([0, 1, 2]),
        };
        assert!(
            Image::parse(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{field}"
        );
    }
    assert!(Image::parse(&vec![0; 512 * 1024 + 1]).is_err());
    assert!(Image::parse(b"not a firmware").is_err());
    let legacy = b"VolooSwitch:CH32X035F8U6:USB-ISP:1\0";
    assert!(matches!(Image::parse(legacy).unwrap(), Image::Legacy(_)));
}
#[test]
fn firmware_frames_are_uid_action_length_and_crc_qualified() {
    let uid: Uid = "01-23-45-67-89-AB-CD-EF".parse().unwrap();
    let request = protocol::firmware_request(uid, 2, &[0; 130]).unwrap();
    assert_eq!(request.frame.len(), 144);
    assert_eq!(protocol::crc16(&request.frame), 0);
    let mut state = vec![1, 0, 0, 1, 255, 0, 1, 0];
    state.resize(32, 0);
    state[21] = 128;
    state[22] = 2;
    let mut reply = vec![0xf7, 0x49];
    reply.extend_from_slice(&uid.0);
    reply.extend_from_slice(&[2, 32]);
    reply.extend_from_slice(&state);
    let reply = protocol::with_crc(reply);
    let mut decoder = FrameDecoder::default();
    let mut frames = Vec::new();
    for chunk in reply.chunks(3) {
        frames.extend(decoder.push(chunk));
    }
    assert_eq!(frames.len(), 1);
    assert_eq!(
        protocol::match_reply(uid, &request.expect, &frames[0]),
        Some(Matched::Firmware(state.clone()))
    );
    let foreign: Uid = "11-22-33-44-55-66-77-88".parse().unwrap();
    assert!(protocol::match_reply(foreign, &request.expect, &reply).is_none());
    let query = protocol::firmware_request(uid, 0, &[]).unwrap();
    assert!(protocol::match_reply(uid, &query.expect, &reply).is_none());
    assert_eq!(AbStatus::decode(&state).unwrap().received, 128);
    state[1] = 2;
    assert!(AbStatus::decode(&state).is_err());
}

#[test]
fn sealed_frames_keep_protocol_identity_and_factory_lock_out_of_public_actions() {
    let uid: Uid = "01-23-45-67-89-AB-CD-EF".parse().unwrap();
    let request = protocol::firmware_request_on(uid, 0x4a, 2, &[0; 146]).unwrap();
    assert_eq!(request.frame.len(), 160);
    assert_eq!(protocol::crc16(&request.frame), 0);
    let mut state = vec![2, 0, 0, 1, 255, 0, 255, 14];
    state.resize(32, 0);
    let mut reply = vec![0xf7, 0x4a];
    reply.extend_from_slice(&uid.0);
    reply.extend_from_slice(&[2, 32]);
    reply.extend_from_slice(&state);
    let reply = protocol::with_crc(reply);
    let mut decoder = FrameDecoder::default();
    let frames: Vec<_> = reply
        .chunks(2)
        .flat_map(|chunk| decoder.push(chunk))
        .collect();
    assert_eq!(frames.len(), 1);
    assert_eq!(
        protocol::match_reply(uid, &request.expect, &frames[0]),
        Some(Matched::Firmware(state.clone()))
    );
    let legacy = protocol::firmware_request(uid, 2, &[]).unwrap();
    assert!(protocol::match_reply(uid, &legacy.expect, &reply).is_none());
    let status = AbStatus::decode(&state).unwrap();
    assert!(status.encrypted);
    assert_eq!(status.protection_flags, 14);
    assert!(protocol::firmware_request_on(uid, 0x4a, 8, &[0; 40]).is_err());
    let mut exception = vec![0xf7, 0xca];
    exception.extend_from_slice(&uid.0);
    exception.push(4);
    assert_eq!(decoder.push(&protocol::with_crc(exception)).len(), 1);
}
