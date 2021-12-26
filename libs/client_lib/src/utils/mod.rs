use serde::de::DeserializeOwned;

pub fn parse_jwt<T: DeserializeOwned>(
    id_token: &str,
) -> jsonwebtoken::errors::Result<jsonwebtoken::TokenData<T>> {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.insecure_disable_signature_validation();
    validation.validate_exp = false;
    jsonwebtoken::decode::<T>(
        id_token,
        &jsonwebtoken::DecodingKey::from_secret(&[]),
        &validation,
    )
}
