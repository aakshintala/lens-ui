use crate::error::{ClientError, Result};
use crate::ids::ConnectionId;
use reqwest::blocking::RequestBuilder;

#[derive(Clone, Debug)]
pub enum Auth {
    None,
    Bearer { token: String },
    Cookie { value: String },
    ForwardedEmail { email: String },
}

impl Auth {
    pub fn apply(&self, rb: RequestBuilder) -> RequestBuilder {
        match self {
            Auth::None => rb,
            Auth::Bearer { token } => rb.bearer_auth(token),
            Auth::Cookie { value } => rb.header(reqwest::header::COOKIE, value),
            Auth::ForwardedEmail { email } => rb.header("X-Forwarded-Email", email),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub id: ConnectionId,
    pub base_url: url::Url,
    pub auth: Auth,
}

impl Connection {
    pub fn new(id: ConnectionId, base_url: url::Url, auth: Auth) -> Self {
        Self { id, base_url, auth }
    }

    /// Join a `/`-rooted absolute path onto the connection's base URL.
    /// Assumes the server is hosted at the URL root (absolute `/`-rooted paths
    /// replace any base path); revisit if a base-path deployment is ever needed.
    pub fn url(&self, path: &str) -> Result<url::Url> {
        self.base_url.join(path).map_err(|e| ClientError::NotFound {
            what: format!("bad url {path}: {e}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ConnectionId;

    #[test]
    fn url_joins_rooted_path() {
        let c = Connection::new(
            ConnectionId::new("c1"),
            "http://localhost:8000".parse().unwrap(),
            Auth::None,
        );
        let u = c.url("/v1/sessions").unwrap();
        assert_eq!(u.as_str(), "http://localhost:8000/v1/sessions");
    }

    #[test]
    fn bearer_auth_sets_authorization_header() {
        // We assert the RequestBuilder carries the header by building a request.
        let client = reqwest::blocking::Client::new();
        let rb = client.get("http://localhost:8000/health");
        let rb = Auth::Bearer {
            token: "tok123".into(),
        }
        .apply(rb);
        let req = rb.build().unwrap();
        assert_eq!(req.headers().get("authorization").unwrap(), "Bearer tok123");
    }

    #[test]
    fn forwarded_email_sets_header() {
        let client = reqwest::blocking::Client::new();
        let rb = client.get("http://localhost:8000/health");
        let rb = Auth::ForwardedEmail {
            email: "a@b.com".into(),
        }
        .apply(rb);
        let req = rb.build().unwrap();
        assert_eq!(req.headers().get("x-forwarded-email").unwrap(), "a@b.com");
    }
}
