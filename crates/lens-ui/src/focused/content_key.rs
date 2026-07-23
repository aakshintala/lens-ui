use lens_core::domain::ids::AccId;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentKey(String);

impl ContentKey {
    pub fn from_acc(acc_id: &AccId) -> Self {
        Self(format!("md:{}", acc_id.as_str()))
    }

    pub fn from_label(label: impl Into<String>) -> Self {
        Self(format!("md:{}", label.into()))
    }

    pub fn as_element_id(&self) -> gpui::SharedString {
        gpui::SharedString::from(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_acc_format() {
        let key = ContentKey::from_acc(&AccId::new("acc_1"));
        assert_eq!(key.0, "md:acc_1");
        assert_eq!(key.as_element_id().as_str(), "md:acc_1");
    }

    #[test]
    fn from_label_format() {
        let key = ContentKey::from_label("user-md-0");
        assert_eq!(key.0, "md:user-md-0");
    }
}
