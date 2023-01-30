extern crate pretty_env_logger;
#[macro_use]
extern crate log;

mod globals;
mod graph_receiver;
mod graph_store;
mod proto_helpers;
mod query_processor;
mod query_processor_holder;
mod query_realizator;
mod worker_service;

use std::env;
use std::net::ToSocketAddrs;

use local_ip_address::local_ip;
use tonic::transport::Server;

use crate::graph_receiver::GraphReceiver;
use crate::query_processor_holder::QueryProcessorHolder;
use crate::worker_service::WorkerService;

pub struct ErrorCollection {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let num_cpus = num_cpus::get();

    pretty_env_logger::init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // Our "blocking" threads will do CPU-bound tasks (as opposed to IO), so the upper limit
        // should be low (default max is 512).
        .max_blocking_threads(num_cpus - 1)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    use generated::manager::manager_service_client::ManagerServiceClient;
    use generated::worker::worker_server::WorkerServer;

    let manager_addr = env::var("PARTITIONER_IP")?;
    let my_port = env::var("MY_PORT")?;

    info!(
        "got manager ip address in environment variable `PARTITIONER_IP`: {}",
        manager_addr
    );

    info!("connecting to manager");

    let client = ManagerServiceClient::connect(manager_addr)
        .await
        .map_err(|e| format!("Cannot connect to the manager: {:?}", e))?;

    let worker_port = my_port.parse::<u16>()?;
    let my_local_ip = local_ip().unwrap();

    debug!("obtained own IP address: {:?}", my_local_ip);

    let listening_addr = format!("{}:{}", my_local_ip, worker_port)
        .to_socket_addrs()
        .map_err(|e| format!("failed to parse own address: {e:?}"))?
        .next()
        .ok_or_else(|| format!("no own address found"))?;

    let listening_addr_unparsed = format!("http://{}", listening_addr);

    let mut receiver = GraphReceiver::new(client, listening_addr_unparsed).await?;
    receiver.receive_graph().await?;

    globals::PROCESSOR_HOLDER
        .set(QueryProcessorHolder::new())
        .map_err(|_| "Cannot set global processor holder")?;

    let service = WorkerService::new();
    let server = WorkerServer::new(service);

    Server::builder()
        .add_service(server)
        .serve(listening_addr)
        .await?;

    Ok(())
}
