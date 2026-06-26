use serde::{Deserialize, Serialize};

macro_rules! branded_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct $name(String);

            impl $name {
                pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
                pub fn as_str(&self) -> &str { &self.0 }
            }

            impl std::fmt::Display for $name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(&self.0)
                }
            }
        )+
    };
}

branded_id!(
    SessionId,
    ElicitationId,
    HostId,
    RunnerId,
    TerminalId,
    FileId,
    CommentId,
    PolicyId,
    ConnectionId,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_json_and_display() {
        let s = SessionId::new("sess_abc");
        assert_eq!(s.as_str(), "sess_abc");
        assert_eq!(s.to_string(), "sess_abc");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"sess_abc\"");
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn distinct_id_types_do_not_unify() {
        // Compile-time guarantee: this block must not compile if uncommented.
        // let _: SessionId = HostId::new("h"); // <- type error by construction
        assert_ne!(
            std::any::TypeId::of::<SessionId>(),
            std::any::TypeId::of::<HostId>()
        );
    }
}
