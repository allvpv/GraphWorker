extern crate pretty_env_logger;
#[macro_use]
extern crate log;

mod executer_service;
mod query_coordinator;
mod workers_connection;

use tonic::transport::Server;
use std::env;
use std::net::ToSocketAddrs;
use local_ip_address::local_ip;

use generated::executer::executer_server::ExecuterServer;
use generated::manager::manager_service_client::ManagerServiceClient;

use crate::executer_service::ExecuterService;

pub struct ErrorCollection {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();

    println!("connecting to manager");
    let manager_addr = env::var("PARTITIONER_IP").unwrap();
    let mut manager = ManagerServiceClient::connect(manager_addr).await?;
    println!("connected to manager");

    let addresses = workers_connection::get_sorted_workers_addresses(&mut manager).await?;
    let workers = workers_connection::connect_to_all_workers(addresses).await?;

    println!("creating the server");
    let service = ExecuterService::new(workers);
    let server = ExecuterServer::new(service);

    let my_local_ip = local_ip().unwrap();
    println!("This is my local IP address: {:?}", my_local_ip);
    let listening_addr = format!("{}:{}", my_local_ip, 49999).to_socket_addrs().expect("Failed to parse own address").next().expect("No own address found");

    println!("starting server at address: '{}'", listening_addr);
    Server::builder()
        .add_service(server)
        .serve(listening_addr)
        .await?;

    Ok(())
}
