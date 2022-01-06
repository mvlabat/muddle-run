use crate::net::auth::{AuthRequest, OAuthResponse};
use bevy::log;
use tokio::sync::mpsc::UnboundedSender;

pub async fn serve(auth_request_tx: UnboundedSender<AuthRequest>) {
    let auth_request_tx_clone = auth_request_tx.clone();
    let make_svc = hyper::service::make_service_fn(move |_conn| {
        fn bad_request() -> hyper::Response<hyper::Body> {
            hyper::Response::builder()
                .status(400)
                .body("Bad Request".into())
                .unwrap()
        }

        let auth_request_tx = auth_request_tx_clone.clone();
        let serve = move |req: hyper::Request<hyper::Body>| {
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

                Ok::<_, std::convert::Infallible>(hyper::Response::<hyper::Body>::new(
                    include_str!("../../../../bins/web_client/auth/index.html").into(),
                ))
            }
        };

        async move { Ok::<_, std::convert::Infallible>(hyper::service::service_fn(serve)) }
    });

    let addr = ([127, 0, 0, 1], 0).into();

    let server = hyper::Server::bind(&addr).serve(make_svc);

    log::info!(
        "Redirect uri server is listening on http://{}",
        server.local_addr()
    );

    auth_request_tx
        .send(AuthRequest::RedirectUrlServerPort(
            server.local_addr().port(),
        ))
        .expect("Failed to write to a channel (auth request)");

    if let Err(err) = server.await {
        log::error!(
            "An error occurred while serving the redirect uri: {:?}",
            err
        );
    }
}
