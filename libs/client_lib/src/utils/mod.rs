use jwt_compact::{Claims, UntrustedToken};
use mr_utils_lib::JwtAuthClaims;

pub fn parse_jwt(id_token: &str) -> Result<Claims<JwtAuthClaims>, jwt_compact::ParseError> {
    let token = UntrustedToken::new(id_token)?;
    token
        .deserialize_claims_unchecked()
        .map_err(|err| match err {
            jwt_compact::ValidationError::MalformedClaims(err) => {
                jwt_compact::ParseError::MalformedHeader(err)
            }
            _ => unreachable!(),
        })
}

#[cfg(test)]
mod tests {
    use crate::utils::parse_jwt;
    use mr_utils_lib::JwtAuthClaims;

    #[test]
    fn test_parse_jwt() {
        assert_eq!(
            parse_jwt("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IjBIQ3dReE9Fa3lnOUU2RXhDbHpuTyJ9.eyJlbWFpbCI6Im12bGFiYXRAZ21haWwuY29tIiwiZW1haWxfdmVyaWZpZWQiOmZhbHNlLCJpc3MiOiJodHRwczovL211ZGRsZS1ydW4uZXUuYXV0aDAuY29tLyIsInN1YiI6ImF1dGgwfDYxYzM5Mjc3ZDIyNjg5MDA3MTI5NjYzOCIsImF1ZCI6IlVOU05CeW1XUkg5N1JhY3c3aFcyZDl0RE4xZ3NxdmZPIiwiaWF0IjoxNjQwMzQ4MjkxLCJleHAiOjE2NDAzODQyOTF9.dAfIolzTRPadxavnno8F4kL1jClOYvSMyoqv1fYCf_ohFB5yfl7SyRCC1ivubSZRS9veL8nZJXX_fX1e8qUAIPBRhogYcrBTzVpGaKQO8yh20yB1yvPbCfmZKh4l1K3CDnm4MaLBkZumHBKbPR5IN3Y4qv8z5TspRzejZHh8ZuAfF8EKl5VCnLWynglU6Mp2ppdLKBqf_GzIoQB-Lesh9o5b81TSViUFWPZRFFcL3orKMFhHyHviwgSmrmfUfRrNwFTx1o1du2LRDw7noITZusBFhOPDshsF7ppWyChoXv-D3t2uj71Zc6tWFc-njvX0OzTHa-KRk8Ox_FpTS_1Emg").unwrap().custom,
            JwtAuthClaims {
                email: Some("mvlabat@gmail.com".to_owned()),
                iss: "https://muddle-run.eu.auth0.com/".to_owned(),
                sub: "auth0|61c39277d226890071296638".to_owned(),
                aud: "UNSNBymWRH97Racw7hW2d9tDN1gsqvfO".to_owned(),
            },
        );
    }
}
