use ethereum_types::H256;
use sha3::{Digest, Keccak256};

pub fn uns_namehash(domain_name: &str) -> Option<ethereum_types::H256> {
    if domain_name.trim().to_ascii_lowercase() != domain_name {
        return None;
    }

    let mut concatenated_hashes = [0; 64];
    for domain_label in domain_name.split('.').rev() {
        if domain_label.is_empty() {
            continue;
        }

        let mut hasher = Keccak256::new();
        hasher.update(domain_label.as_bytes());
        concatenated_hashes[32..].copy_from_slice(hasher.finalize().as_slice());

        let mut hasher = Keccak256::new();
        hasher.update(&concatenated_hashes[..]);
        concatenated_hashes[..32].copy_from_slice(hasher.finalize().as_slice());
    }

    let mut res: [u8; 32] = [0; 32];
    res.copy_from_slice(&concatenated_hashes[..32]);
    Some(H256(res))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_namehash(domain_name: &str, expected_hash: &str) {
        assert_eq!(
            hex::encode(uns_namehash(domain_name).unwrap()),
            expected_hash
        );
    }

    #[test]
    fn test_top_level_domain() {
        assert_namehash(
            "crypto",
            "0f4a10a4f46c288cea365fcf45cccf0e9d901b945b9829ccdb54c10dc3cb7a6f",
        );
    }

    #[test]
    fn test_second_level_domain() {
        assert_namehash(
            "rustacean.crypto",
            "a0e2ed57ebc6fa203973214b94605b2ae6968eb7bdc51d049caf8ecb425c45d1",
        );
    }
}
