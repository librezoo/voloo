//! Byte-exact protocol vectors (tests/fixtures/protocol-examples.json).
use serde_json::Value;
use voloo::protocol::*;

fn bytes(hex: &str) -> Vec<u8> {
    hex.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}
fn vectors() -> Value {
    serde_json::from_str(include_str!("fixtures/protocol-examples.json")).unwrap()
}
fn action<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["examples"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["actions"].as_array().unwrap())
        .find(|a| a["name"] == name)
        .unwrap()
}

#[test]
fn uid_parse_and_format() {
    let uid: Uid = "01:23:45:67:89:ab:cd:ef".parse().unwrap();
    assert_eq!(uid.to_string(), "01-23-45-67-89-AB-CD-EF");
    assert_eq!("0123456789ABCDEF".parse::<Uid>().unwrap(), uid);
    assert!("0123".parse::<Uid>().is_err());
    assert!("0123456789ABCDEG".parse::<Uid>().is_err());
}

#[test]
fn every_vector_has_a_valid_crc_and_decodes() {
    let v = vectors();
    let uid: Uid = v["uid"].as_str().unwrap().parse().unwrap();
    let mut decoded = 0;
    for a in v["examples"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["actions"].as_array().unwrap())
    {
        // Broadcast discovery (FC41/42) is not used on point-to-point USB.
        let Some(req) = a["request_hex"].as_str().map(bytes) else {
            continue;
        };
        if matches!(req[1], 0x41 | 0x42) {
            continue;
        }
        let n = req.len();
        assert_eq!(
            crc16(&req[..n - 2]).to_le_bytes(),
            [req[n - 2], req[n - 1]],
            "{}",
            a["name"]
        );
        let Some(resp) = a["response_hex"].as_str() else {
            continue;
        };
        let resp = bytes(resp);
        let frames = FrameDecoder::default().push(&resp);
        assert_eq!(frames, vec![resp.clone()], "{}", a["name"]);
        if let Some(values) = a["register_values"].as_array() {
            let fc = req[1];
            let count = values.len() as u16;
            let m = match_reply(uid, &Expect::Registers { fc, count }, &resp).unwrap();
            let expected: Vec<u16> = values.iter().map(|x| x.as_u64().unwrap() as u16).collect();
            assert_eq!(m, Matched::Registers(expected), "{}", a["name"]);
            decoded += 1;
        }
    }
    assert_eq!(
        decoded, 5,
        "status, control, identity, runtime and ADC vectors"
    );
}

#[test]
fn builders_match_vectors() {
    let v = vectors();
    let uid: Uid = v["uid"].as_str().unwrap().parse().unwrap();
    let req = |name| bytes(action(&v, name)["request_hex"].as_str().unwrap());
    assert_eq!(read_input(uid, 0, 6).unwrap().frame, req("read_status"));
    assert_eq!(
        read_input(uid, 0x100, 9).unwrap().frame,
        req("read_identity")
    );
    assert_eq!(
        read_input(uid, 0x200, 8).unwrap().frame,
        req("read_runtime_diagnostics")
    );
    assert_eq!(
        read_input(uid, 0x300, 13).unwrap().frame,
        req("read_adc_diagnostics")
    );
    assert_eq!(
        read_input(uid, 0x110, 13).unwrap().frame,
        req("read_nickname_after_write")
    );
    assert_eq!(write_control(uid, Command::On).frame, req("command_on"));
    assert_eq!(write_control(uid, Command::Off).frame, req("command_off"));
    assert_eq!(
        write_control(uid, Command::Discharge).frame,
        req("command_discharge")
    );
    assert_eq!(
        write_nickname(uid, "PWR-01").unwrap().frame,
        req("set_nickname")
    );
    assert_eq!(
        write_nickname(uid, "").unwrap().frame,
        req("clear_nickname")
    );
    // Echo expectations match the documented replies.
    for (name, r) in [
        ("set_nickname", write_nickname(uid, "PWR-01").unwrap()),
        ("command_on", write_control(uid, Command::On)),
    ] {
        let resp = bytes(action(&v, name)["response_hex"].as_str().unwrap());
        assert_eq!(
            match_reply(uid, &r.expect, &resp),
            Some(Matched::Echo),
            "{name}"
        );
    }
}

#[test]
fn exception_and_foreign_replies() {
    let v = vectors();
    let uid: Uid = v["uid"].as_str().unwrap().parse().unwrap();
    let resp = bytes(
        action(&v, "illegal_address")["response_hex"]
            .as_str()
            .unwrap(),
    );
    let m = match_reply(uid, &Expect::Registers { fc: 4, count: 1 }, &resp).unwrap();
    assert_eq!(m, Matched::Exception(DeviceException { fc: 4, code: 2 }));
    let other: Uid = "FF-23-45-67-89-AB-CD-EF".parse().unwrap();
    assert_eq!(
        match_reply(other, &Expect::Registers { fc: 4, count: 1 }, &resp),
        None
    );
    // A status-shaped reply does not satisfy an identity read.
    let status = bytes(action(&v, "read_status")["response_hex"].as_str().unwrap());
    assert_eq!(
        match_reply(uid, &Expect::Registers { fc: 4, count: 9 }, &status),
        None
    );
}

#[test]
fn decoder_resynchronizes_after_noise_and_bad_crc() {
    let v = vectors();
    let good = bytes(action(&v, "read_status")["response_hex"].as_str().unwrap());
    let mut bad = good.clone();
    *bad.last_mut().unwrap() ^= 0xFF;
    let mut d = FrameDecoder::default();
    let mut stream = vec![0x00, 0x13];
    stream.extend(&bad);
    stream.extend(&good[..7]);
    assert!(d.push(&stream).is_empty());
    assert_eq!(d.push(&good[7..]), vec![good.clone()]);
    assert!(d.crc_errors >= 1);
}

#[test]
fn status_decode_reports_invalid_measurements_as_none() {
    let s = Status::decode(&[0, 0x0000, 0, 0, 0, 2]).unwrap();
    assert_eq!((s.vin_mv, s.vout_mv), (None, None));
    let s = Status::decode(&[1, 25, 5000, 5000, 1, 2]).unwrap();
    assert_eq!(
        (
            s.power_state.as_str(),
            s.vin_mv,
            s.vout_mv,
            s.output_enabled
        ),
        ("ON", Some(5000), Some(5000), true)
    );
    let id = Identity::decode(&[517, 1, 519, 256, 291, 17767, 35243, 52719, 1023]).unwrap();
    assert_eq!(
        (
            id.uid.to_string().as_str(),
            id.protocol_version.as_str(),
            id.firmware_version.as_str()
        ),
        ("01-23-45-67-89-AB-CD-EF", "2.5", "2.7")
    );
}

#[test]
fn input_validation() {
    let uid: Uid = "0123456789ABCDEF".parse().unwrap();
    assert!(read_input(uid, 0x0004, 3).is_err(), "crosses a gap");
    assert!(read_input(uid, 0x0300, 14).is_err());
    assert!(read_input(uid, 0x0006, 1).is_err());
    assert!(nickname_data(&"电".repeat(9)).is_err(), "27 bytes");
    assert!(nickname_data("a\u{7}").is_err());
    assert!(nickname_data("电源🔌").is_ok());
    let data = nickname_data("电源🔌").unwrap();
    let regs: Vec<u16> = data
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(decode_nickname(&regs).unwrap(), "电源🔌");
}
