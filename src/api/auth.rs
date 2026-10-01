//! OAuth 2.0 Authorization Code + PKCE for installed apps ("Desktop app"
//! client type): open consent URL in browser, receive the code on a loopback
//! listener `http://127.0.0.1:<random-port>`, exchange it via `oauth2` crate.
//! Scope: `https://www.googleapis.com/auth/youtube.readonly`.
