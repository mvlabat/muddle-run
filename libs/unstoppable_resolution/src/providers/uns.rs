use crate::namehash::uns_namehash;
use ethabi::StateMutability;
use reqwest::{Client, Url};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use snafu::Snafu;
use std::{collections::HashMap, sync::Arc};

const MAINNET_UNS_PROXY_READER: &str = "0xc3c2bab5e3e52dbf311b2aacef2e40344f19494e";
const POLYGON_UNS_PROXY_READER: &str = "0xa3f32c8cd786dc089bd1fc175f2707223aee5d00";

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Default)]
pub struct JrdLink {
    /// URI.
    pub rel: String,
    /// URI used for looking it up if rel is not some URL.
    pub href: Option<Url>,
    /// Media type (MIME type).
    /// See: https://developer.mozilla.org/en-US/docs/Web/HTTP/Basics_of_HTTP/MIME_types.
    pub mime_type: Option<String>,
    /// Mapping of language to title, with default being a fallback language.
    /// Uses language tags. See: https://datatracker.ietf.org/doc/html/rfc1766.
    #[serde(default)]
    pub titles: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Default)]
pub struct JrdDocument {
    /// URI describing the subject of the document.
    pub subject: String,
    /// URIs the document might also refer to.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Mapping of URIs to arbitrary strings.
    #[serde(default)]
    pub properties: HashMap<String, String>,
    /// List of JRDLink objects.
    #[serde(default)]
    pub links: Vec<JrdLink>,
    /// Date at witch this document expires.
    pub expires: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct WebFingerRecord {
    pub host: Option<String>,
    pub uri: Option<String>,
    #[serde(default)]
    pub value: JsonValue,
}

#[derive(Snafu, Debug)]
pub enum ResolutionError {
    RequestError {
        error: reqwest::Error,
        kind: RequestErrorKind,
    },
    JsonRpcError {
        error: JsonRpcErrorObject,
    },
    InvalidResult {
        result: Option<JsonValue>,
    },
    UnregisteredDomainError,
}

mod web_finger {
    use crate::{RequestErrorKind, ResolutionError};
    use reqwest::Url;
    use serde_json::Value as JsonValue;
    use snafu::Snafu;

    #[derive(Snafu, Debug)]
    pub enum WebFingerResponseError {
        RequestError {
            error: reqwest::Error,
            kind: RequestErrorKind,
        },
        InvalidDomainName,
        InvalidRecord {
            key: String,
            value: String,
        },
        InvalidResult {
            result: Option<JsonValue>,
        },
        DomainResolutionError {
            source: ResolutionError,
        },
        SchemeNotSupported {
            url: Url,
        },
    }
}
pub use web_finger::WebFingerResponseError;

#[derive(Debug)]
pub enum RequestErrorKind {
    FailedRequest,
    InvalidJsonResponse,
}

#[allow(unused)]
#[derive(Deserialize, Debug)]
pub struct JsonRpcErrorObject {
    code: isize,
    message: String,
    data: Option<JsonValue>,
}

#[allow(unused)]
#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: usize,
    result: Option<JsonValue>,
    error: Option<JsonRpcErrorObject>,
}

#[derive(Clone)]
pub struct UnsResolutionProvider {
    pub http_client: Client,
    pub ethereum_rpc_url: Arc<Url>,
    pub polygon_rpc_url: Arc<Url>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ResolutionData {
    pub resolver: ethabi::Address,
    pub owner: ethabi::Address,
    pub values: Vec<String>,
}

impl UnsResolutionProvider {
    pub fn new(ethereum_rpc_url: Url, polygon_rpc_url: Url) -> Self {
        Self {
            http_client: Client::new(),
            ethereum_rpc_url: Arc::new(ethereum_rpc_url),
            polygon_rpc_url: Arc::new(polygon_rpc_url),
        }
    }

    pub async fn domain_jrd(
        &self,
        domain_name: &str,
        user: &str,
        rel: &str,
        fallback_issuer: Option<Url>,
    ) -> Result<JrdDocument, WebFingerResponseError> {
        let Some(domain_namehash) = uns_namehash(domain_name) else {
            return Err(WebFingerResponseError::InvalidDomainName);
        };
        let web_finger_key = format!("webfinger.{}.{}", user, rel);
        let data = self
            .data(&domain_namehash, vec![web_finger_key.clone()])
            .await
            .map_err(|err| WebFingerResponseError::DomainResolutionError { source: err })?;

        let resource = if user.is_empty() {
            format!("acct:{}@{}", user, domain_name)
        } else {
            domain_name.to_owned()
        };

        let web_finger_record_value = &data.values[0];
        if web_finger_record_value.is_empty() {
            return Ok(JrdDocument {
                subject: resource,
                links: vec![JrdLink {
                    rel: rel.to_owned(),
                    href: fallback_issuer.or_else(|| {
                        Some(Url::parse("https://auth.unstoppabledomains.com").unwrap())
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            });
        }

        let invalid_record_err = WebFingerResponseError::InvalidRecord {
            key: web_finger_key,
            value: web_finger_record_value.to_owned(),
        };
        let Ok(web_finger_record) = serde_json::from_str::<WebFingerRecord>(web_finger_record_value) else {
            return Err(invalid_record_err);
        };

        let json: JrdDocument = if let Some(host) = web_finger_record.host {
            let Ok(mut url) = Url::parse(&host).and_then(|host| host.join(".well-known/webfinger")) else {
                return Err(invalid_record_err);
            };
            url.query_pairs_mut()
                .append_pair("resource", &resource)
                .append_pair("rel", rel);
            reqwest::get(url)
                .await
                .map_err(|err| WebFingerResponseError::RequestError {
                    error: err,
                    kind: RequestErrorKind::FailedRequest,
                })?
                .json()
                .await
                .map_err(|err| WebFingerResponseError::RequestError {
                    error: err,
                    kind: RequestErrorKind::InvalidJsonResponse,
                })?
        } else if let Some(uri) = web_finger_record.uri {
            let Ok(url) = Url::parse(&uri) else {
                return Err(invalid_record_err);
            };
            match url.scheme() {
                "http" | "https" => reqwest::get(url)
                    .await
                    .map_err(|err| WebFingerResponseError::RequestError {
                        error: err,
                        kind: RequestErrorKind::FailedRequest,
                    })?
                    .json()
                    .await
                    .map_err(|err| WebFingerResponseError::RequestError {
                        error: err,
                        kind: RequestErrorKind::InvalidJsonResponse,
                    })?,
                _ => {
                    return Err(WebFingerResponseError::SchemeNotSupported { url });
                }
            }
        } else {
            serde_json::from_value(web_finger_record.value.clone()).map_err(|_| {
                WebFingerResponseError::InvalidResult {
                    result: Some(web_finger_record.value),
                }
            })?
        };

        Ok(json)
    }

    pub async fn data(
        &self,
        domain_namehash: &ethereum_types::H256,
        keys: Vec<String>,
    ) -> Result<ResolutionData, ResolutionError> {
        let zero_address = ethabi::Address::from([0; 20]);

        let mainnet_data = self
            .proxy_reader_data(
                &*self.ethereum_rpc_url,
                MAINNET_UNS_PROXY_READER,
                domain_namehash,
                keys.clone(),
            )
            .await?;
        if mainnet_data.owner != zero_address {
            return Ok(mainnet_data);
        }
        let polygon_data = self
            .proxy_reader_data(
                &*self.polygon_rpc_url,
                POLYGON_UNS_PROXY_READER,
                domain_namehash,
                keys,
            )
            .await?;
        if polygon_data.owner == zero_address {
            return Err(ResolutionError::UnregisteredDomainError);
        }
        Ok(polygon_data)
    }

    pub async fn proxy_reader_data(
        &self,
        rpc: &Url,
        proxy_reader_address: &str,
        domain_namehash: &ethereum_types::H256,
        keys: Vec<String>,
    ) -> Result<ResolutionData, ResolutionError> {
        #[allow(deprecated)]
        let get_data_fn = ethabi::Function {
            name: "getData".to_owned(),
            inputs: vec![
                ethabi::Param {
                    name: "keys".to_owned(),
                    kind: ethabi::ParamType::Array(Box::new(ethabi::ParamType::String)),
                    internal_type: Some("string[]".to_owned()),
                },
                ethabi::Param {
                    name: "tokenId".to_owned(),
                    kind: ethabi::ParamType::Uint(256),
                    internal_type: Some("uint256".to_owned()),
                },
            ],
            outputs: vec![
                ethabi::Param {
                    name: "resolver".to_owned(),
                    kind: ethabi::ParamType::Address,
                    internal_type: Some("address".to_owned()),
                },
                ethabi::Param {
                    name: "owner".to_owned(),
                    kind: ethabi::ParamType::Address,
                    internal_type: Some("address".to_owned()),
                },
                ethabi::Param {
                    name: "values".to_owned(),
                    kind: ethabi::ParamType::Array(Box::new(ethabi::ParamType::String)),
                    internal_type: Some("string[]".to_owned()),
                },
            ],
            state_mutability: StateMutability::View,
            constant: true,
        };

        let keys_array =
            ethabi::Token::Array(keys.into_iter().map(ethabi::Token::String).collect());

        let (resolver, owner, records) = {
            let mut result = self
                .eth_call(
                    rpc,
                    proxy_reader_address,
                    &get_data_fn,
                    &[
                        keys_array,
                        ethabi::Token::Uint(namehash_to_u256(domain_namehash)),
                    ],
                )
                .await?;

            // Popping result tokens in the reverse order.
            let Some(ethabi::Token::Array(tokens)) = result.pop() else {
                return Err(ResolutionError::InvalidResult { result: None });
            };
            let Some(ethabi::Token::Address(owner)) = result.pop() else {
                return Err(ResolutionError::InvalidResult { result: None });
            };
            let Some(ethabi::Token::Address(resolver)) = result.pop() else {
                return Err(ResolutionError::InvalidResult { result: None });
            };
            (resolver, owner, tokens)
        };
        let records = records
            .into_iter()
            .map(|token| {
                if let ethabi::Token::String(value) = token {
                    value
                } else {
                    unreachable!()
                }
            })
            .collect();

        Ok(ResolutionData {
            resolver,
            owner,
            values: records,
        })
    }

    async fn eth_call(
        &self,
        rpc: &Url,
        contract_address: &str,
        function: &ethabi::Function,
        arguments: &[ethabi::Token],
    ) -> Result<Vec<ethabi::Token>, ResolutionError> {
        let data = function.encode_input(arguments).unwrap();
        let data = "0x".to_owned() + &hex::encode(data);

        let response = self
            .http_client
            .post((*rpc).clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "eth_call",
                "params": [
                    {
                        "from": "0x0000000000000000000000000000000000000000",
                        "data": data,
                        "to": contract_address
                    },
                    "latest"
                ]
            }))
            .send()
            .await
            .map_err(|err| ResolutionError::RequestError {
                error: err,
                kind: RequestErrorKind::FailedRequest,
            })?;

        let json_response: JsonRpcResponse =
            response
                .json()
                .await
                .map_err(|err| ResolutionError::RequestError {
                    error: err,
                    kind: RequestErrorKind::InvalidJsonResponse,
                })?;

        if let Some(error) = json_response.error {
            return Err(ResolutionError::JsonRpcError { error });
        }

        json_response
            .result
            .as_ref()
            .and_then(|result| result.as_str())
            .and_then(|result| hex::decode(result.get(2..)?).ok())
            .and_then(|result| function.decode_output(&result).ok())
            .ok_or(ResolutionError::InvalidResult {
                result: json_response.result,
            })
    }
}

fn namehash_to_u256(domain_namehash: &ethereum_types::H256) -> ethereum_types::U256 {
    ethereum_types::U256::from_big_endian(&domain_namehash.0)
}
