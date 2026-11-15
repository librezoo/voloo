use serde_json::{Value, json};
use voloo::{
    firmware::{AbStatus, Image, inspect},
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

fn sealed_v1() -> Value {
    let mut header = vec![0; 48];
    header[..4].copy_from_slice(b"VSE1");
    header[4..8].copy_from_slice(&[3, 0, 3, 0]);
    header[8..12].copy_from_slice(&[2, 0, 2, 0]);
    header[32] = 1;
    json!({"format":"voloo-x035-sealed-v1","hardware_version":0x300,"firmware_version":0x300,"third_party_notices":"Monocypher license",
        "header":header,"cipher_a":vec![0u8;576],"cipher_b":vec![0u8;576],
        "recipients":[{"uid":"01-02-03-04-05-06-07-08","nonce":vec![0u8;24],"wrapped_key":vec![0u8;32],"tag":vec![0u8;16]}]})
}

fn sealed_v2() -> Value {
    let key_id = 0x1234_5678u32;
    let mut header = vec![0; 48];
    header[..4].copy_from_slice(b"VSE2");
    header[4..8].copy_from_slice(&[3, 0, 3, 1]);
    header[8..12].copy_from_slice(&[2, 0, 2, 0]);
    header[32] = 1;
    let mut nonce = key_id.to_le_bytes().to_vec();
    nonce.extend_from_slice(&[0xA5; 20]);
    json!({"format":"voloo-x035-sealed-v2","hardware_version":0x300,"firmware_version":0x301,"third_party_notices":"Monocypher license",
        "header":header,"cipher_a":vec![0u8;576],"cipher_b":vec![0u8;576],
        "universal":{"key_id":key_id,"nonce":nonce,"wrapped_key":vec![0x5A;32],"tag":vec![0xC3;16]}})
}

#[test]
fn sealed_v1_and_universal_v2_packages_validate_distinct_wire_shapes() {
    assert!(matches!(
        Image::parse(&serde_json::to_vec(&sealed_v1()).unwrap()).unwrap(),
        Image::Sealed(_)
    ));
    assert!(matches!(
        Image::parse(&serde_json::to_vec(&sealed_v2()).unwrap()).unwrap(),
        Image::Sealed(_)
    ));

    let mut missing = sealed_v2();
    missing.as_object_mut().unwrap().remove("universal");
    assert!(Image::parse(&serde_json::to_vec(&missing).unwrap()).is_err());

    let mut mismatch = sealed_v2();
    mismatch["universal"]["nonce"][0] = json!(0);
    assert!(Image::parse(&serde_json::to_vec(&mismatch).unwrap()).is_err());

    let mut mixed_v2 = sealed_v2();
    mixed_v2["recipients"] = json!([]);
    assert!(Image::parse(&serde_json::to_vec(&mixed_v2).unwrap()).is_err());

    let mut plaintext_v2 = sealed_v2();
    plaintext_v2["slot_a"] = json!([1, 2, 3]);
    assert!(Image::parse(&serde_json::to_vec(&plaintext_v2).unwrap()).is_err());

    let mut leaked_envelope_key = sealed_v2();
    leaked_envelope_key["universal"]["key"] = json!("00");
    assert!(Image::parse(&serde_json::to_vec(&leaked_envelope_key).unwrap()).is_err());

    let mut leaked_product_key = sealed_v2();
    leaked_product_key["upgrade_key"] = json!("00");
    assert!(Image::parse(&serde_json::to_vec(&leaked_product_key).unwrap()).is_err());

    let mut mixed_v1 = sealed_v1();
    mixed_v1["universal"] = json!({});
    assert!(Image::parse(&serde_json::to_vec(&mixed_v1).unwrap()).is_err());

    let mut old_version = sealed_v2();
    old_version["firmware_version"] = json!(0x300);
    old_version["header"][7] = json!(0);
    assert!(Image::parse(&serde_json::to_vec(&old_version).unwrap()).is_err());

    let plain = serde_json::to_vec(&bundle(0x301)).unwrap();
    assert!(inspect(&plain).is_err());
}

#[test]
fn status_byte_three_means_universal_key_encrypted_firmware() {
    let mut state = vec![3, 0, 0, 1, 255, 0, 255, 14];
    state.resize(32, 0);
    let status = AbStatus::decode(&state).unwrap();
    assert!(status.encrypted);
    assert!(status.universal);
    assert_eq!(status.protection_flags, 14);
}
