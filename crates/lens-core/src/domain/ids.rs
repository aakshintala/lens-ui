//! Branded ids. Reuses `lens-client`'s 9 ids (public newtypes, full serde) and
//! adds the 4 engine-local ones. `lens-client`'s `branded_id!` macro is not
//! exported, so we define a local one — trivial and keeps the crates decoupled.

use serde::{Deserialize, Serialize};

// Re-export the ids that already live in lens-client (§2.1 reuse boundary).
pub use lens_client::ids::{
    CommentId, ConnectionId, ElicitationId, FileId, HostId, PolicyId, RunnerId, SessionId,
    TerminalId,
};

macro_rules! branded_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

// Engine-local ids not present in lens-client (§2.1). BridgeItemId is Bridge
// scope (§11) — out of this spec.
branded_id!(
    ItemId,
    CallId,
    ResponseId,
    AgentId,
    BoardId,
    BoardItemId,
    AccId
);

impl ResponseId {
    /// Normalize wire `response_id`: absent or empty string → `None`.
    pub fn from_wire(s: Option<&str>) -> Option<Self> {
        s.filter(|s| !s.is_empty()).map(|s| Self::new(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_roundtrips_json_and_display() {
        let id = ItemId::new("item_abc");
        assert_eq!(id.as_str(), "item_abc");
        assert_eq!(id.to_string(), "item_abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"item_abc\"");
        let back: ItemId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn reexported_id_is_usable() {
        // Proves the re-export path compiles and the type is constructible here.
        let s = SessionId::new("conv_1");
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
