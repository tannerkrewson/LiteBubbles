//! Parse the manual hardware payload produced by OpenBubbles' Mac Hardware Info.
//!
//! The accepted input is the base64 encoding of these bytes:
//!
//! ```text
//! ASCII "OABS" | sharing flag (0 or 1) | bbhwinfo.HwInfo protobuf
//! ```
//!
//! The protobuf contains the software fields used by rustpush's macOS
//! configuration and every field required by rustpush's `HardwareConfig`.
//! Both values of the flag are accepted because the Mac helper puts the same
//! payload behind its QR code regardless of the privacy checkbox.  An `MB...`
//! sharing code is deliberately rejected: accepting one would require a
//! network request to the OpenBubbles sharing service, which is outside this
//! parser's contract.
//!
//! A genuine Mac is used only to create this activation input.  LiteBubbles
//! does not contact the Mac or use it as a relay after the payload has been
//! entered.  The caller must keep the parsed value in the backend's protected
//! setup state; this module never logs or formats hardware values.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use prost::Message;
use thiserror::Error;

const PREFIX: &[u8; 4] = b"OABS";
const HEADER_LEN: usize = PREFIX.len() + 1;
const MAX_INPUT_BYTES: usize = 128 * 1024;
const MAX_PROTOBUF_BYTES: usize = 96 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_OPAQUE_BYTES: usize = 4096;

/// A validated hardware payload with all values kept on the backend side.
#[derive(Clone, PartialEq, Eq)]
pub struct MacHardwareInput {
    software: MacSoftwareInfo,
    hardware: MacHardwareConfig,
    sharing_prevented: bool,
}

impl MacHardwareInput {
    /// Parse the base64 text copied from Mac Hardware Info.
    pub fn parse_base64(input: &str) -> Result<Self, HardwareInputError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(HardwareInputError::EmptyInput);
        }
        if input.starts_with("MB") {
            return Err(HardwareInputError::SharingCodeUnsupported);
        }
        if input.len() > MAX_INPUT_BYTES {
            return Err(HardwareInputError::InputTooLarge);
        }

        let payload = STANDARD
            .decode(input)
            .map_err(|_| HardwareInputError::InvalidBase64)?;
        Self::parse_payload(&payload)
    }

    /// Parse decoded `OABS` bytes.  This is useful for a QR reader that has
    /// already decoded the payload; it does not accept MB sharing codes.
    pub fn parse_payload(payload: &[u8]) -> Result<Self, HardwareInputError> {
        if payload.len() > MAX_PROTOBUF_BYTES + HEADER_LEN {
            return Err(HardwareInputError::PayloadTooLarge);
        }
        if payload.len() < HEADER_LEN + 1 {
            return Err(HardwareInputError::Truncated);
        }
        if &payload[..PREFIX.len()] != PREFIX {
            return Err(HardwareInputError::InvalidPrefix);
        }

        let sharing_prevented = match payload[PREFIX.len()] {
            0 => false,
            1 => true,
            _ => return Err(HardwareInputError::InvalidSharingFlag),
        };

        let wire = WireHwInfo::decode(&payload[HEADER_LEN..])
            .map_err(|_| HardwareInputError::InvalidProtobuf)?;
        Self::from_wire(wire, sharing_prevented)
    }

    /// Return the software fields required to construct rustpush's macOS
    /// configuration.
    pub fn software(&self) -> &MacSoftwareInfo {
        &self.software
    }

    /// Return the hardware fields required by rustpush's `HardwareConfig`.
    pub fn hardware(&self) -> &MacHardwareConfig {
        &self.hardware
    }

    /// Whether the Mac helper marked this payload as preventing sharing.
    pub fn sharing_prevented(&self) -> bool {
        self.sharing_prevented
    }

    fn from_wire(wire: WireHwInfo, sharing_prevented: bool) -> Result<Self, HardwareInputError> {
        let inner = wire
            .inner
            .ok_or(HardwareInputError::MissingField("inner"))?;

        let software = MacSoftwareInfo {
            version: required_text(&wire.version, "version")?,
            protocol_version: positive_protocol_version(wire.protocol_version)?,
            device_id: required_text(&wire.device_id, "device_id")?,
            icloud_ua: required_text(&wire.icloud_ua, "icloud_ua")?,
            aoskit_version: required_text(&wire.aoskit_version, "aoskit_version")?,
        };

        let hardware = MacHardwareConfig {
            product_name: required_text(&inner.product_name, "product_name")?,
            io_mac_address: fixed_bytes::<6>(&inner.io_mac_address, "io_mac_address")?,
            platform_serial_number: required_text(
                &inner.platform_serial_number,
                "platform_serial_number",
            )?,
            platform_uuid: required_text(&inner.platform_uuid, "platform_uuid")?,
            root_disk_uuid: required_text(&inner.root_disk_uuid, "root_disk_uuid")?,
            board_id: required_text(&inner.board_id, "board_id")?,
            os_build_num: required_text(&inner.os_build_num, "os_build_num")?,
            platform_serial_number_enc: required_bytes(
                &inner.platform_serial_number_enc,
                "platform_serial_number_enc",
            )?,
            platform_uuid_enc: required_bytes(&inner.platform_uuid_enc, "platform_uuid_enc")?,
            root_disk_uuid_enc: required_bytes(&inner.root_disk_uuid_enc, "root_disk_uuid_enc")?,
            rom: required_bytes(&inner.rom, "rom")?,
            rom_enc: required_bytes(&inner.rom_enc, "rom_enc")?,
            mlb: required_text(&inner.mlb, "mlb")?,
            mlb_enc: required_bytes(&inner.mlb_enc, "mlb_enc")?,
        };

        Ok(Self {
            software,
            hardware,
            sharing_prevented,
        })
    }
}

impl std::fmt::Debug for MacHardwareInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacHardwareInput")
            .field("software", &Redacted)
            .field("hardware", &Redacted)
            .field("sharing_prevented", &self.sharing_prevented)
            .finish()
    }
}

/// The top-level software values from `bbhwinfo.HwInfo`.
#[derive(Clone, PartialEq, Eq)]
pub struct MacSoftwareInfo {
    version: String,
    protocol_version: u32,
    device_id: String,
    icloud_ua: String,
    aoskit_version: String,
}

impl MacSoftwareInfo {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn icloud_ua(&self) -> &str {
        &self.icloud_ua
    }

    pub fn aoskit_version(&self) -> &str {
        &self.aoskit_version
    }
}

impl std::fmt::Debug for MacSoftwareInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacSoftwareInfo")
            .field("version", &Redacted)
            .field("protocol_version", &Redacted)
            .field("device_id", &Redacted)
            .field("icloud_ua", &Redacted)
            .field("aoskit_version", &Redacted)
            .finish()
    }
}

/// The inner hardware values required by rustpush's `HardwareConfig`.
#[derive(Clone, PartialEq, Eq)]
pub struct MacHardwareConfig {
    product_name: String,
    io_mac_address: [u8; 6],
    platform_serial_number: String,
    platform_uuid: String,
    root_disk_uuid: String,
    board_id: String,
    os_build_num: String,
    platform_serial_number_enc: Vec<u8>,
    platform_uuid_enc: Vec<u8>,
    root_disk_uuid_enc: Vec<u8>,
    rom: Vec<u8>,
    rom_enc: Vec<u8>,
    mlb: String,
    mlb_enc: Vec<u8>,
}

impl MacHardwareConfig {
    pub fn product_name(&self) -> &str {
        &self.product_name
    }

    pub fn io_mac_address(&self) -> &[u8; 6] {
        &self.io_mac_address
    }

    pub fn platform_serial_number(&self) -> &str {
        &self.platform_serial_number
    }

    pub fn platform_uuid(&self) -> &str {
        &self.platform_uuid
    }

    pub fn root_disk_uuid(&self) -> &str {
        &self.root_disk_uuid
    }

    pub fn board_id(&self) -> &str {
        &self.board_id
    }

    pub fn os_build_num(&self) -> &str {
        &self.os_build_num
    }

    pub fn platform_serial_number_enc(&self) -> &[u8] {
        &self.platform_serial_number_enc
    }

    pub fn platform_uuid_enc(&self) -> &[u8] {
        &self.platform_uuid_enc
    }

    pub fn root_disk_uuid_enc(&self) -> &[u8] {
        &self.root_disk_uuid_enc
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn rom_enc(&self) -> &[u8] {
        &self.rom_enc
    }

    pub fn mlb(&self) -> &str {
        &self.mlb
    }

    pub fn mlb_enc(&self) -> &[u8] {
        &self.mlb_enc
    }
}

impl std::fmt::Debug for MacHardwareConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacHardwareConfig")
            .field("product_name", &Redacted)
            .field("io_mac_address", &Redacted)
            .field("platform_serial_number", &Redacted)
            .field("platform_uuid", &Redacted)
            .field("root_disk_uuid", &Redacted)
            .field("board_id", &Redacted)
            .field("os_build_num", &Redacted)
            .field("platform_serial_number_enc", &Redacted)
            .field("platform_uuid_enc", &Redacted)
            .field("root_disk_uuid_enc", &Redacted)
            .field("rom", &Redacted)
            .field("rom_enc", &Redacted)
            .field("mlb", &Redacted)
            .field("mlb_enc", &Redacted)
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HardwareInputError {
    #[error("hardware input is empty")]
    EmptyInput,
    #[error(
        "MB sharing codes are not supported; paste the base64 payload from Mac Hardware Info instead"
    )]
    SharingCodeUnsupported,
    #[error("hardware input is too large")]
    InputTooLarge,
    #[error("hardware payload is too large")]
    PayloadTooLarge,
    #[error("hardware input is not valid standard base64")]
    InvalidBase64,
    #[error("hardware payload is truncated")]
    Truncated,
    #[error("hardware payload has an invalid OABS prefix")]
    InvalidPrefix,
    #[error("hardware payload has an invalid sharing flag")]
    InvalidSharingFlag,
    #[error("hardware payload contains invalid protobuf data")]
    InvalidProtobuf,
    #[error("hardware payload is missing required field {0}")]
    MissingField(&'static str),
    #[error("hardware field {field} has an invalid value")]
    InvalidField { field: &'static str },
    #[error("hardware field {field} has invalid byte length {actual}")]
    InvalidByteLength { field: &'static str, actual: usize },
}

fn required_text(value: &str, field: &'static str) -> Result<String, HardwareInputError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(HardwareInputError::InvalidField { field });
    }
    Ok(value.to_owned())
}

fn required_bytes(value: &[u8], field: &'static str) -> Result<Vec<u8>, HardwareInputError> {
    if value.is_empty() {
        return Err(HardwareInputError::MissingField(field));
    }
    if value.len() > MAX_OPAQUE_BYTES {
        return Err(HardwareInputError::InvalidByteLength {
            field,
            actual: value.len(),
        });
    }
    Ok(value.to_vec())
}

fn fixed_bytes<const N: usize>(
    value: &[u8],
    field: &'static str,
) -> Result<[u8; N], HardwareInputError> {
    value
        .try_into()
        .map_err(|_| HardwareInputError::InvalidByteLength {
            field,
            actual: value.len(),
        })
}

fn positive_protocol_version(value: i32) -> Result<u32, HardwareInputError> {
    if value <= 0 {
        return Err(HardwareInputError::InvalidField {
            field: "protocol_version",
        });
    }
    Ok(value as u32)
}

struct Redacted;

impl std::fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Clone, PartialEq, Message)]
struct WireHwInfo {
    #[prost(message, optional, tag = "1")]
    inner: Option<WireInnerHwInfo>,
    #[prost(string, tag = "2")]
    version: String,
    #[prost(int32, tag = "3")]
    protocol_version: i32,
    #[prost(string, tag = "4")]
    device_id: String,
    #[prost(string, tag = "5")]
    icloud_ua: String,
    #[prost(string, tag = "6")]
    aoskit_version: String,
}

#[derive(Clone, PartialEq, Message)]
struct WireInnerHwInfo {
    #[prost(string, tag = "1")]
    product_name: String,
    #[prost(bytes = "vec", tag = "2")]
    io_mac_address: Vec<u8>,
    #[prost(string, tag = "3")]
    platform_serial_number: String,
    #[prost(string, tag = "4")]
    platform_uuid: String,
    #[prost(string, tag = "5")]
    root_disk_uuid: String,
    #[prost(string, tag = "6")]
    board_id: String,
    #[prost(string, tag = "7")]
    os_build_num: String,
    #[prost(bytes = "vec", tag = "8")]
    platform_serial_number_enc: Vec<u8>,
    #[prost(bytes = "vec", tag = "9")]
    platform_uuid_enc: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    root_disk_uuid_enc: Vec<u8>,
    #[prost(bytes = "vec", tag = "11")]
    rom: Vec<u8>,
    #[prost(bytes = "vec", tag = "12")]
    rom_enc: Vec<u8>,
    #[prost(string, tag = "13")]
    mlb: String,
    #[prost(bytes = "vec", tag = "14")]
    mlb_enc: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_wire() -> WireHwInfo {
        WireHwInfo {
            inner: Some(WireInnerHwInfo {
                product_name: "Synthetic-Mac".to_owned(),
                io_mac_address: vec![0x10, 0x20, 0x30, 0x40, 0x50, 0x60],
                platform_serial_number: "SYNTHETIC-SERIAL".to_owned(),
                platform_uuid: "synthetic-platform-uuid".to_owned(),
                root_disk_uuid: "synthetic-root-disk-uuid".to_owned(),
                board_id: "Synthetic-Board".to_owned(),
                os_build_num: "SyntheticBuild".to_owned(),
                platform_serial_number_enc: vec![1, 2, 3, 4],
                platform_uuid_enc: vec![5, 6, 7, 8],
                root_disk_uuid_enc: vec![9, 10, 11, 12],
                rom: vec![13, 14, 15, 16, 17, 18],
                rom_enc: vec![19, 20, 21, 22],
                mlb: "SYNTHETIC-MLB".to_owned(),
                mlb_enc: vec![23, 24, 25, 26],
            }),
            version: "Synthetic macOS".to_owned(),
            protocol_version: 1640,
            device_id: "synthetic-device-id".to_owned(),
            icloud_ua: "synthetic-icloud-ua".to_owned(),
            aoskit_version: "synthetic-aoskit-version".to_owned(),
        }
    }

    fn synthetic_payload(flag: u8) -> Vec<u8> {
        let mut protobuf = Vec::new();
        synthetic_wire().encode(&mut protobuf).unwrap();

        let mut payload = PREFIX.to_vec();
        payload.push(flag);
        payload.extend(protobuf);
        payload
    }

    fn synthetic_base64(flag: u8) -> String {
        STANDARD.encode(synthetic_payload(flag))
    }

    #[test]
    fn parses_manual_base64_payload() {
        let parsed = MacHardwareInput::parse_base64(&synthetic_base64(0)).unwrap();

        assert!(!parsed.sharing_prevented());
        assert_eq!(parsed.software().protocol_version(), 1640);
        assert_eq!(
            parsed.hardware().io_mac_address(),
            &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]
        );
        assert_eq!(parsed.hardware().rom(), &[13, 14, 15, 16, 17, 18]);
    }

    #[test]
    fn accepts_the_prevent_sharing_flag_without_network_access() {
        let parsed = MacHardwareInput::parse_base64(&synthetic_base64(1)).unwrap();

        assert!(parsed.sharing_prevented());
    }

    #[test]
    fn rejects_mb_sharing_codes_explicitly() {
        assert_eq!(
            MacHardwareInput::parse_base64("MBABCD-EFGH-IJKL-MNOP"),
            Err(HardwareInputError::SharingCodeUnsupported)
        );
    }

    #[test]
    fn rejects_malformed_base64() {
        assert_eq!(
            MacHardwareInput::parse_base64("not base64!"),
            Err(HardwareInputError::InvalidBase64)
        );
    }

    #[test]
    fn rejects_truncated_payloads() {
        let payload = b"OABS\0";

        assert_eq!(
            MacHardwareInput::parse_payload(payload),
            Err(HardwareInputError::Truncated)
        );
    }

    #[test]
    fn rejects_invalid_prefix_and_flag() {
        let mut invalid_prefix = synthetic_payload(0);
        invalid_prefix[0] = b'X';
        assert_eq!(
            MacHardwareInput::parse_payload(&invalid_prefix),
            Err(HardwareInputError::InvalidPrefix)
        );

        let invalid_flag = synthetic_payload(2);
        assert_eq!(
            MacHardwareInput::parse_payload(&invalid_flag),
            Err(HardwareInputError::InvalidSharingFlag)
        );
    }

    #[test]
    fn rejects_invalid_protobuf_and_trailing_invalid_bytes() {
        let mut payload = synthetic_payload(0);
        payload.push(0xff);
        assert_eq!(
            MacHardwareInput::parse_payload(&payload),
            Err(HardwareInputError::InvalidProtobuf)
        );

        let mut truncated_protobuf = PREFIX.to_vec();
        truncated_protobuf.push(0);
        truncated_protobuf.extend([0x0a, 0x05, 0x01]);
        assert_eq!(
            MacHardwareInput::parse_payload(&truncated_protobuf),
            Err(HardwareInputError::InvalidProtobuf)
        );
    }

    #[test]
    fn rejects_missing_and_wrong_length_fields() {
        let mut missing_version = synthetic_wire();
        missing_version.version.clear();
        let mut protobuf = Vec::new();
        missing_version.encode(&mut protobuf).unwrap();
        let mut payload = PREFIX.to_vec();
        payload.push(0);
        payload.extend(protobuf);
        assert_eq!(
            MacHardwareInput::parse_payload(&payload),
            Err(HardwareInputError::InvalidField { field: "version" })
        );

        let mut wrong_mac_length = synthetic_wire();
        wrong_mac_length
            .inner
            .as_mut()
            .unwrap()
            .io_mac_address
            .pop();
        let mut protobuf = Vec::new();
        wrong_mac_length.encode(&mut protobuf).unwrap();
        let mut payload = PREFIX.to_vec();
        payload.push(0);
        payload.extend(protobuf);
        assert_eq!(
            MacHardwareInput::parse_payload(&payload),
            Err(HardwareInputError::InvalidByteLength {
                field: "io_mac_address",
                actual: 5
            })
        );

        let mut invalid_protocol_version = synthetic_wire();
        invalid_protocol_version.protocol_version = 0;
        let mut protobuf = Vec::new();
        invalid_protocol_version.encode(&mut protobuf).unwrap();
        let mut payload = PREFIX.to_vec();
        payload.push(0);
        payload.extend(protobuf);
        assert_eq!(
            MacHardwareInput::parse_payload(&payload),
            Err(HardwareInputError::InvalidField {
                field: "protocol_version"
            })
        );
    }

    #[test]
    fn redacts_hardware_and_software_values_from_debug() {
        let parsed = MacHardwareInput::parse_base64(&synthetic_base64(0)).unwrap();
        let debug = format!("{parsed:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("SYNTHETIC"));
        assert!(!debug.contains("synthetic"));
        assert!(!debug.contains("10, 32, 48"));
    }
}
