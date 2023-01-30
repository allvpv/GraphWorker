import argparse
import grpc

import grpc
import executer_pb2
import executer_pb2_grpc


def main():
    parser = argparse.ArgumentParser(description='Test the implementation')
    parser.add_argument("--from-node", type=int, help="",required=True)
    parser.add_argument("--to-node", type=int, help="",required=True)
    parser.add_argument("--executer-addr", help="", required=True)
    args = parser.parse_args()

    print("Trying to get shortest path from {} to {} ...".format(args.from_node, args.to_node))
    with grpc.insecure_channel(args.executer_addr) as channel:
        stub = executer_pb2_grpc.ExecuterStub(channel)
        response = stub.ShortestPathQuery(executer_pb2.QueryData(node_id_from=args.from_node, node_id_to=args.to_node))
        print(f"Shortest path length received: {response.shortest_path_len}")
        print("Path found:")
        for node in stub.BacktrackPathForQuery(executer_pb2.QueryId(query_id=response.query_id)):
            print(node.node_id, node.worker_id)


if __name__ == "__main__":
    main()
