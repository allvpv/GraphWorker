use std::sync::atomic::{AtomicU32, Ordering};
use tonic::{Request, Response, Result};

use generated::executer::executer_server::Executer;
use generated::executer::{QueryData, QueryFinished};

use crate::query_coordinator::QueryCoordinator;
use crate::workers_connection::Worker;

pub type NodeId = u64;
pub type ShortestPathLen = u64;

pub struct ExecuterService {
    workers: Vec<Worker>,
    query_id_counter: AtomicU32,
}

impl ExecuterService {
    pub fn new(workers: Vec<Worker>) -> Self {
        ExecuterService {
            workers,
            query_id_counter: AtomicU32::new(0),
        }
    }

    fn get_new_query_id(&self) -> u32 {
        self.query_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_forget_query(mut coordinator: QueryCoordinator) {
        match coordinator.send_forget_to_workers().await {
            Err(e) => warn!("Cannot send forget query to workers: {e}"),
            Ok(()) => (),
        }
    }
}

#[tonic::async_trait]
impl Executer for ExecuterService {
    async fn shortest_path_query(
        &self,
        request: Request<QueryData>,
    ) -> Result<Response<QueryFinished>> {
        let QueryData {
            node_id_from,
            node_id_to,
        } = request.into_inner();

        let query_id = self.get_new_query_id();
        info!("`query_id` is: {query_id}");

        if node_id_from == node_id_to {
            Ok(Response::new(QueryFinished {
                shortest_path_len: Some(0),
            }))
        } else {
            let mut coordinator =
                QueryCoordinator::new(&self.workers, node_id_from, node_id_to, query_id).await?;
            let response = coordinator.shortest_path_query().await?;

            tokio::spawn(Self::send_forget_query(coordinator));

            Ok(Response::new(response))
        }
    }
}
