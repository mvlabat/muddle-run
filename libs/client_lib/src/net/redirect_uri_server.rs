use crate::net::auth::{AuthRequest, OAuthResponse};
use bevy::log;
use tokio::sync::mpsc::UnboundedSender;

pub async fn serve(auth_request_tx: UnboundedSender<AuthRequest>) {
    let auth_request_tx_clone = auth_request_tx.clone();
    let make_svc = || {
        let auth_request_tx = auth_request_tx_clone.clone();

        hyper::service::service_fn(move |req| {
            fn bad_request() -> hyper::Response<String> {
                hyper::Response::builder()
                    .status(400)
                    .body("Bad Request".into())
                    .unwrap()
            }

            let auth_request_tx = auth_request_tx.clone();

            async move {
                let uri = req.uri();
                // Ignore favicon requests and stuff.
                if uri.path().len() > 1 {
                    return Ok(bad_request());
                }

                let Some(params) = uri.query() else {
                    log::error!("Invalid OAuth response: missing query params");
                    return Ok(bad_request());
                };

                let OAuthResponse { state, code } = match serde_urlencoded::from_str(params) {
                    Ok(params) => params,
                    Err(err) => {
                        log::error!("Failed to parse OAuth server response: {:?}", err);
                        return Ok(bad_request());
                    }
                };

                auth_request_tx
                    .send(AuthRequest::HandleOAuthResponse { state, code })
                    .expect("Failed to write to a channel (auth request)");

                Ok::<_, std::convert::Infallible>(hyper::Response::<String>::new(
                    include_str!("../../../../bins/web_client/auth/index.html").into(),
                ))
            }
        })
    };

    let addr = (std::net::Ipv4Addr::new(127, 0, 0, 1), 0);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to initialise the redirect uri server");

    let local_addr = listener
        .local_addr()
        .expect("Failed to initialise the redirect uri server");
    log::info!("Redirect uri server is listening on http://{local_addr}");

    auth_request_tx
        .send(AuthRequest::RedirectUrlServerPort(local_addr.port()))
        .expect("Failed to write to a channel (auth request)");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(r) => r,
            Err(err) => {
                log::warn!("Redirect URI server failed to accept a connection: {err:?}");
                continue;
            }
        };

        let service = make_svc();
        tokio::spawn(async move {
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(stream, service)
                .await
            {
                log::error!("Error serving redirect URI server connection: {err:?}");
            }
        });
    }
}
