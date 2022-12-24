pub mod worker {
    tonic::include_proto!("worker");
}

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::stream::Stream;
use tonic::{Request, Response, Status};

use worker::worker_server::Worker;
use worker::{IsPresent, NodeId as NodeIdProto, RequestDjikstra, ResponseDjikstra};

use crate::graph_store::NodeMapping;
use crate::graph_store::SPQGraph;
use crate::request_server::{RequestId, RequestServer};

#[derive(Debug)]
enum RequestServerHolder {
    Busy, // The request is pending, RequestServer was moved to blocking thread
    Ready(RequestServer),
}

type RequestIdServerMap = HashMap<RequestId, RequestServerHolder>;

#[derive(Debug)]
pub struct WorkerService {
    graph: Arc<SPQGraph>,
    mapping: Arc<NodeMapping>,
    requests: Mutex<RequestIdServerMap>,
}

impl WorkerService {
    pub fn new(graph: SPQGraph, mapping: NodeMapping) -> Self {
        WorkerService {
            graph: Arc::new(graph),
            mapping: Arc::new(mapping),
            requests: Mutex::new(RequestIdServerMap::new()),
        }
    }
}

#[tonic::async_trait]
impl Worker for WorkerService {
    async fn is_node_present(
        &self,
        request: Request<NodeIdProto>,
    ) -> Result<Response<IsPresent>, Status> {
        let node_id = request.get_ref().node_id;
        let present = self.mapping.contains_key(&node_id);

        Ok(Response::new(IsPresent { present }))
    }

    type UpdateDjikstraStream =
        Pin<Box<dyn Stream<Item = Result<ResponseDjikstra, Status>> + Send + 'static>>;

    async fn update_djikstra(
        &self,
        request: Request<tonic::Streaming<RequestDjikstra>>,
    ) -> Result<Response<Self::UpdateDjikstraStream>, Status> {
        let mut inbound = request.into_inner();

        use crate::worker_service::worker::request_djikstra::RequestType::RequestId as ProtoRequestId;

        let next_message = inbound.message().await?.map(|r| r.request_type).flatten();

        let request_id: RequestId = match next_message {
            Some(ProtoRequestId(id)) => id,
            _ => {
                return Err(Status::invalid_argument(
                    "First message in UpdateDjikstra stream must be request_id",
                ))
            }
        };

        let mutex_error = |e| Status::internal(format!("Internal error while locking mutex: {e}"));
        let duplicate_request_error = || {
            Status::invalid_argument(
                "Error. Executer requested UpdateDjikstra, while another \
                 UpdateDjikstra on this query was already pending",
            )
        };

        use std::collections::hash_map::Entry::{Occupied, Vacant};

        let mut request_server = {
            match self.requests.lock().map_err(mutex_error)?.entry(request_id) {
                Vacant(entry) => {
                    entry.insert(RequestServerHolder::Busy);
                    RequestServer::new(self.graph.clone(), self.mapping.clone())
                }
                Occupied(mut entry) => match entry.insert(RequestServerHolder::Busy) {
                    RequestServerHolder::Busy => return Err(duplicate_request_error()),
                    RequestServerHolder::Ready(server) => server,
                },
            }
        };

        request_server.apply_update(&mut inbound).await?;

        // We move the server in and out the task to satisfy borrow checker
        let (request_server, result_vec) =
            tokio::task::spawn_blocking(move || request_server.djikstra_step())
                .await
                .expect("RequestServer task panicked")?;
        let result_iter = result_vec.into_iter().map(|s| Ok(s));
        let output = futures::stream::iter(result_iter);

        // Give back server to the RequestServerHolder
        if let Some(holder) = self
            .requests
            .lock()
            .map_err(mutex_error)?
            .get_mut(&request_id)
        {
            *holder = RequestServerHolder::Ready(request_server);
        } else {
            unreachable!();
        }

        Ok(Response::new(Box::pin(output) as Self::UpdateDjikstraStream))
    }
}
