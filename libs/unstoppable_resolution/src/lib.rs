#![feature(let_else)]

mod providers;

pub mod namehash;
pub use ethabi;
pub use providers::uns::*;

#[cfg(test)]
mod tests {
    use reqwest::Url;
    use std::str::FromStr;

    use crate::{namehash::uns_namehash, providers::uns::ResolutionData, UnsResolutionProvider};

    #[tokio::test]
    async fn fetches_data() {
        let provider = UnsResolutionProvider::new(
            Url::parse("https://mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39").unwrap(),
            Url::parse("https://polygon-mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39")
                .unwrap(),
        );

        let records = provider
            .data(
                &uns_namehash("reseller-test-udtesting-620535166920.crypto").unwrap(),
                vec![
                    "crypto.ETH.address".to_owned(),
                    "crypto.BTC.address".to_owned(),
                ],
            )
            .await
            .unwrap();

        let expected_resolution_data = ResolutionData {
            resolver: ethabi::Address::from_str("0xa9a6a3626993d487d2dbda3173cf58ca1a9d9e9f")
                .unwrap(),
            owner: ethabi::Address::from_str("0x9ccd5fe18dd4e0947b41fb460462c03904607e55").unwrap(),
            values: vec![
                "0x87348226e747df4cff2b1b1e38a528df405ccd5c".to_owned(),
                "".to_owned(),
            ],
        };

        assert_eq!(records, expected_resolution_data,);
    }
}
