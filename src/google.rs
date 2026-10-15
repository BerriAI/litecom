//! Google OpenID Connect, following LiteReview's authorization-code + PKCE flow.
use crate::{
    crypto::secret,
    error::{Error, Result, validate},
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, IdToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{
        CoreClient, CoreGenderClaim, CoreIdTokenVerifier, CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm, CoreProviderMetadata,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Google {
    pub client_id: String,
    client_secret: String,
    allowed_domains: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct PendingGoogle {
    pub nonce: String,
    pub verifier: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub subject: String,
    pub email: String,
    pub name: String,
    pub audience: String,
    pub domain: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GoogleClaims {
    #[serde(default)]
    hd: Option<String>,
}
impl openidconnect::AdditionalClaims for GoogleClaims {}

type GoogleIdToken = IdToken<
    GoogleClaims,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;

fn invalid() -> Error {
    Error::Invalid("Google sign-in could not be verified. Return to Litecom and try again.".into())
}

impl Google {
    pub fn restricts_workspace(&self) -> bool {
        !self.allowed_domains.is_empty()
    }
    pub fn permits(&self, identity: &Identity) -> bool {
        identity.audience == self.client_id
            && (self.allowed_domains.is_empty()
                || identity.domain.as_ref().is_some_and(|domain| {
                    self.allowed_domains.contains(domain)
                        && identity
                            .email
                            .rsplit_once('@')
                            .is_some_and(|(_, value)| value == domain)
                }))
    }
    pub fn new(client_id: String, client_secret: String, allowed_domains: &str) -> Self {
        Self {
            client_id,
            client_secret,
            allowed_domains: allowed_domains
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    pub fn begin(&self, origin: &str, state: &str) -> (url::Url, PendingGoogle) {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let nonce = secret();
        let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .expect("constant Google URL");
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            (
                "redirect_uri",
                &format!("{origin}/api/auth/google/callback"),
            ),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state),
            ("nonce", &nonce),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        if self.allowed_domains.len() == 1 {
            url.query_pairs_mut()
                .append_pair("hd", &self.allowed_domains[0]);
        }
        (
            url,
            PendingGoogle {
                nonce,
                verifier: verifier.secret().clone(),
            },
        )
    }

    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        origin: &str,
        code: String,
        pending: &PendingGoogle,
    ) -> Result<Identity> {
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new("https://accounts.google.com".into()).expect("constant Google issuer"),
            http,
        )
        .await
        .map_err(|_| {
            Error::Unavailable("Google could not be reached. Try signing in again.".into())
        })?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        )
        .set_redirect_uri(
            RedirectUrl::new(format!("{origin}/api/auth/google/callback"))
                .map_err(|_| invalid())?,
        );
        let response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| invalid())?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.verifier.clone()))
            .request_async(http)
            .await
            .map_err(|_| invalid())?;
        let token = response.id_token().ok_or_else(invalid)?;
        self.verify(
            &token.to_string(),
            &client.id_token_verifier(),
            &pending.nonce,
        )
    }

    fn verify(&self, raw: &str, verifier: &CoreIdTokenVerifier, nonce: &str) -> Result<Identity> {
        let token: GoogleIdToken = raw.parse().map_err(|_| invalid())?;
        let verifier = verifier
            .clone()
            .set_allowed_algs([CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256]);
        let claims = token
            .claims(&verifier, &Nonce::new(nonce.into()))
            .map_err(|_| invalid())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_secs() as i64;
        validate(
            claims.issue_time().timestamp() <= now + 60,
            "Google returned a sign-in from the future. Check your server's clock.",
        )?;
        validate(
            claims.email_verified() == Some(true),
            "Google must verify your email address before you sign in.",
        )?;
        if claims
            .authorized_party()
            .is_some_and(|v| v.as_str() != self.client_id)
            || (claims.audiences().len() > 1 && claims.authorized_party().is_none())
        {
            return Err(invalid());
        }
        let email = claims.email().ok_or_else(invalid)?.as_str().to_lowercase();
        if !self.allowed_domains.is_empty() {
            let domain = claims
                .additional_claims()
                .hd
                .as_deref()
                .unwrap_or_default()
                .to_lowercase();
            validate(
                self.allowed_domains.contains(&domain)
                    && email
                        .rsplit_once('@')
                        .is_some_and(|(_, value)| value == domain),
                "Use a Google account from this server's approved workspace.",
            )?;
        }
        let subject = claims.subject().as_str().to_owned();
        validate(
            !subject.is_empty(),
            "Google did not return an account identity. Try signing in again.",
        )?;
        let name = claims
            .name()
            .and_then(|v| v.get(None))
            .map(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(&email)
            .to_owned();
        Ok(Identity {
            subject,
            email,
            name,
            audience: self.client_id.clone(),
            domain: claims
                .additional_claims()
                .hd
                .as_ref()
                .map(|v| v.to_lowercase()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openidconnect::{
        PrivateSigningKey,
        core::{CoreJsonWebKeySet, CoreRsaPrivateSigningKey},
    };
    use serde_json::json;

    #[test]
    fn signed_google_tokens_require_identity_nonce_audience_domain_and_expiry() {
        // Deliberately public, generated test key. Never used by the application.
        let key = CoreRsaPrivateSigningKey::from_pem(
            include_str!("../scripts/fixtures/oidc-test-key.pem"),
            None,
        )
        .unwrap();
        let google = Google::new("client".into(), "secret".into(), "example.com");
        let verifier = CoreIdTokenVerifier::new_public_client(
            ClientId::new("client".into()),
            IssuerUrl::new("https://accounts.google.com".into()).unwrap(),
            CoreJsonWebKeySet::new(vec![key.as_verification_key()]),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let base = json!({"iss":"https://accounts.google.com","sub":"stable-subject","aud":"client","exp":now+600,"iat":now,"nonce":"expected","email":"moe@example.com","email_verified":true,"name":"Moe","hd":"example.com"});
        let verify = |claims: serde_json::Value, nonce: &str| {
            let claims = serde_json::from_value(claims).unwrap();
            let token = GoogleIdToken::new(
                claims,
                &key,
                CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
                None,
                None,
            )
            .unwrap();
            google.verify(&token.to_string(), &verifier, nonce)
        };
        let identity = verify(base.clone(), "expected").unwrap();
        assert_eq!(identity.subject, "stable-subject");
        assert!(google.permits(&identity));
        assert!(
            !Google::new("different-client".into(), "secret".into(), "example.com")
                .permits(&identity)
        );
        assert!(!Google::new("client".into(), "secret".into(), "other.com").permits(&identity));
        assert!(verify(base.clone(), "wrong").is_err());
        for (field, value) in [
            ("iss", json!("https://evil.example")),
            ("aud", json!("other-client")),
            ("exp", json!(now - 1)),
            ("iat", json!(now + 3600)),
            ("email_verified", json!(false)),
            ("hd", json!("elsewhere.com")),
            ("email", json!("moe@elsewhere.com")),
            ("azp", json!("other-client")),
            ("aud", json!(["client", "other-client"])),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert!(
                verify(changed, "expected").is_err(),
                "accepted invalid {field}"
            );
        }
        let (url, pending) = google.begin("https://litecom.example", "state");
        let params = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["nonce"], pending.nonce);
        assert_eq!(params["state"], "state");
        assert!(!url.as_str().contains("secret"));
    }
}
