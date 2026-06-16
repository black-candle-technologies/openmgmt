pub mod auth;
pub mod error;
pub mod messages;
pub mod sync;
pub mod version;

pub use auth::*;
pub use error::*;
pub use messages::*;
pub use sync::*;
pub use version::*;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn sync_event() -> SyncEvent {
        SyncEvent {
            event_id: "event-1".into(),
            device_id: "device-1".into(),
            actor_user_id: Some("user-a".into()),
            target_user_id: Some("user-b".into()),
            workspace_id: Some("workspace-1".into()),
            sequence: 1,
            entity_type: SyncEntityType::Task,
            entity_id: "task-1".into(),
            operation: SyncOperation::Created,
            payload_json: json!({"entity": {"id": "task-1", "title": "Review request"}}),
            created_at: Utc.with_ymd_and_hms(2026, 6, 13, 12, 0, 0).unwrap(),
            synced_at: None,
        }
    }

    #[test]
    fn protocol_version_compatibility_is_exact() {
        assert!(is_compatible_protocol_version("omgp/1"));
        assert!(!is_compatible_protocol_version("omgp/2"));
        assert!(!is_compatible_protocol_version("OMGP/1"));
    }

    #[test]
    fn hello_request_roundtrips_as_json() {
        let request = SyncHelloRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            client_name: "OpenMGMT Desktop".into(),
            client_version: Some("0.1.0".into()),
            device_id: Some("device-1".into()),
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: SyncHelloRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn sync_push_request_with_event_roundtrips_as_json() {
        let request = SyncPushRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            auth: AuthContext {
                account_id: Some("account-1".into()),
                user_id: Some("user-a".into()),
                device_id: "device-1".into(),
                device_token: None,
            },
            base_checkpoint: Some("checkpoint-1".into()),
            events: vec![sync_event()],
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: SyncPushRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn protocol_message_uses_tagged_envelope() {
        let message = ProtocolMessage::HelloRequest(SyncHelloRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            client_name: "OpenMGMT Desktop".into(),
            client_version: None,
            device_id: None,
        });

        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(value["type"], "hello_request");
        assert_eq!(value["payload"]["protocol_version"], "omgp/1");
        assert_eq!(value["payload"]["client_name"], "OpenMGMT Desktop");
    }

    #[test]
    fn protocol_error_constructors_set_code_and_retryability() {
        let invalid = ProtocolError::invalid_request("missing events");
        assert_eq!(invalid.code, ProtocolErrorCode::InvalidRequest);
        assert!(!invalid.retryable);

        let incompatible = ProtocolError::incompatible_version("omgp/2");
        assert_eq!(incompatible.code, ProtocolErrorCode::IncompatibleVersion);
        assert!(!incompatible.retryable);

        let server = ProtocolError::server_error("temporary failure");
        assert_eq!(server.code, ProtocolErrorCode::ServerError);
        assert!(server.retryable);
    }

    #[test]
    fn sync_event_multi_user_fields_survive_json_roundtrip() {
        let event = sync_event();

        let json = serde_json::to_string(&event).unwrap();
        let decoded: SyncEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.actor_user_id.as_deref(), Some("user-a"));
        assert_eq!(decoded.target_user_id.as_deref(), Some("user-b"));
        assert_eq!(decoded.workspace_id.as_deref(), Some("workspace-1"));
    }
}
