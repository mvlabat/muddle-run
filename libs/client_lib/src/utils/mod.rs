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

#[cfg(test)]
mod tests {
    use crate::utils::parse_jwt;
    use serde::Deserialize;

    #[test]
    fn test_parse_jwt() {
        #[derive(Debug, Eq, PartialEq, Deserialize)]
        struct Claims {
            email: String,
            email_verified: bool,
            iss: String,
            sub: String,
            aud: String,
            iat: i64,
            exp: i64,
            at_hash: Option<String>,
        }

        assert_eq!(
            parse_jwt::<Claims>("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IjBIQ3dReE9Fa3lnOUU2RXhDbHpuTyJ9.eyJlbWFpbCI6Im12bGFiYXRAZ21haWwuY29tIiwiZW1haWxfdmVyaWZpZWQiOmZhbHNlLCJpc3MiOiJodHRwczovL211ZGRsZS1ydW4uZXUuYXV0aDAuY29tLyIsInN1YiI6ImF1dGgwfDYxYzM5Mjc3ZDIyNjg5MDA3MTI5NjYzOCIsImF1ZCI6IlVOU05CeW1XUkg5N1JhY3c3aFcyZDl0RE4xZ3NxdmZPIiwiaWF0IjoxNjQwMzQ4MjkxLCJleHAiOjE2NDAzODQyOTF9.dAfIolzTRPadxavnno8F4kL1jClOYvSMyoqv1fYCf_ohFB5yfl7SyRCC1ivubSZRS9veL8nZJXX_fX1e8qUAIPBRhogYcrBTzVpGaKQO8yh20yB1yvPbCfmZKh4l1K3CDnm4MaLBkZumHBKbPR5IN3Y4qv8z5TspRzejZHh8ZuAfF8EKl5VCnLWynglU6Mp2ppdLKBqf_GzIoQB-Lesh9o5b81TSViUFWPZRFFcL3orKMFhHyHviwgSmrmfUfRrNwFTx1o1du2LRDw7noITZusBFhOPDshsF7ppWyChoXv-D3t2uj71Zc6tWFc-njvX0OzTHa-KRk8Ox_FpTS_1Emg").unwrap().claims,
            Claims {
                email: "mvlabat@gmail.com".to_owned(),
                email_verified: false,
                iss: "https://muddle-run.eu.auth0.com/".to_owned(),
                sub: "auth0|61c39277d226890071296638".to_owned(),
                aud: "UNSNBymWRH97Racw7hW2d9tDN1gsqvfO".to_owned(),
                iat: 1640348291,
                exp: 1640384291,
                at_hash: None,
            },
        );
    }
}
