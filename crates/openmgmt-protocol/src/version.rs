pub const PROTOCOL_NAME: &str = "OpenMGMT Protocol";
pub const PROTOCOL_VERSION: &str = "omgp/1";
pub const MIN_COMPATIBLE_VERSION: &str = "omgp/1";

pub fn is_compatible_protocol_version(value: &str) -> bool {
    value == PROTOCOL_VERSION
}
