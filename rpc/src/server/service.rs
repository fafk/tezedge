use hyper::{Body, Response, Error, Server, Request, StatusCode, Method};
use hyper::service::{service_fn, make_service_fn};
use std::net::SocketAddr;
use futures::Future;
use crate::rpc_actor::RpcServerRef;
use riker::actors::ActorSystem;
use crate::server::ask::ask;
use serde_json;
use chrono::prelude::*;
use crate::encoding::base_types::*;
use tezos_encoding::hash::{HashEncoding, HashType};
use crate::encoding::monitor::BootstrapInfo;

type ServiceResult = Result<Response<Body>, Box<dyn std::error::Error + Sync + Send>>;

/// Spawn new HTTP server on given address interacting with specific actor system
pub fn spawn_server(addr: &SocketAddr, sys: ActorSystem, actor: RpcServerRef) -> impl Future<Output=Result<(), Error>> {
    Server::bind(addr)
        .serve(make_service_fn(move |_| {
            let sys = sys.clone();
            let actor = actor.clone();
            async move {
                let sys = sys.clone();
                let actor = actor.clone();
                Ok::<_, Error>(service_fn(move |req| {
                    let sys = sys.clone();
                    let actor = actor.clone();
                    async move {
                        router(req, sys, actor).await
                    }
                }))
            }
        }))
}

/// Helper function for generating current TimeStamp
#[allow(dead_code)]
fn timestamp() -> TimeStamp {
    TimeStamp::Integral(Utc::now().timestamp())
}

fn ts_to_rfc3339(ts: i64) -> String {
    Utc.from_utc_datetime(&NaiveDateTime::from_timestamp(ts, 0))
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Generate 404 response
fn not_found() -> ServiceResult {
    Ok(Response::builder()
        .status(StatusCode::from_u16(404)?)
        .body(Body::from("not found"))?)
}

/// Generate empty response
fn empty() -> ServiceResult {
    Ok(Response::builder()
        .status(StatusCode::from_u16(204)?)
        .body(Body::empty())?)
}

/// GET /monitor/boostrapped endpoint handler
async fn bootstrapped(sys: ActorSystem, actor: RpcServerRef) -> ServiceResult {
    use crate::server::control_msg::GetCurrentHead;
    use shell::shell_channel::BlockApplied;

    let current_head = ask(&sys, &actor, GetCurrentHead::Request).await;
    if let GetCurrentHead::Response(current_head) = current_head {
        let resp = serde_json::to_string(&if current_head.is_some() {
            let current_head: BlockApplied = current_head.unwrap();
            let block = HashEncoding::new(HashType::BlockHash).bytes_to_string(&current_head.hash);
            let timestamp = ts_to_rfc3339(current_head.header.timestamp());
            BootstrapInfo::new(block.into(), TimeStamp::Rfc(timestamp))
        } else {
            BootstrapInfo::new(String::new().into(), TimeStamp::Integral(0))
        })?;
        Ok(Response::new(Body::from(resp)))
    } else {
        empty()
    }
}

/// GET /monitor/commit_hash endpoint handler
async fn commit_hash(_sys: ActorSystem, _actor: RpcServerRef) -> ServiceResult {
    let resp = serde_json::to_string(&UniString::from(env!("GIT_HASH")))?;
    Ok(Response::new(Body::from(resp)))
}

/// Simple endpoint routing handler
async fn router(req: Request<Body>, sys: ActorSystem, actor: RpcServerRef) -> ServiceResult {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/monitor/bootstrapped") => bootstrapped(sys, actor).await,
        (&Method::GET, "/monitor/commit_hash") => commit_hash(sys, actor).await,
        _ => not_found()
    }
}