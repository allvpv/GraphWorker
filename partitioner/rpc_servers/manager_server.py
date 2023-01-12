import grpc
import re
import threading
import traceback

from google.protobuf import empty_pb2 as google_dot_protobuf_dot_empty__pb2
from partitioners import is_in_partition
from parsers.open_street_map import OpenStreetMapParser
import rpc_servers.manager_pb2 as manager__pb2
import rpc_servers.manager_pb2_grpc as manager_pb2_grpc


def get_node_partition(node, partitions, parser):
    for _, line in parser.get_lines():
        if parser.is_node_line(line):
            node_id, lat, lon = parser.get_node_info(line)
            if node_id != node:
                continue
            return lat, lon, next(filter(lambda p: is_in_partition(lon, lat, p[1]), enumerate(partitions)))

    return None


def get_edge_info(node1, node2, node_cache, parser, partitions, partition_ix):
    if node1 not in node_cache or node_cache[node1][2] != partition_ix:
        return
    lat1, lon1, partition1 = node_cache.get(node1)

    if node2 in node_cache:
        lat2, lon2, partition2 = node_cache.get(node2)
    else:
        lat2, lon2, partition2 = get_node_partition(node2, partitions, parser)
        partition2 = partition2[0]
        node_cache[node2] = (lat2, lon2, partition2)

    edge_weight = ((lat2 - lat1) ** 2 + (lon2 - lon1) ** 2) ** 0.5
    edge_weight = int(1e7 * edge_weight)
    if partition2 != partition1:
        return manager__pb2.Edge(
            node_from_id=node1,
            node_to_id=node2,
            weight=edge_weight,
            node_to_worker_id=partition2 + 1
        )

    return manager__pb2.Edge(
        node_from_id=node1,
        node_to_id=node2,
        weight=edge_weight,
    )


def send_partition_edges(node_cache, partitions, parser):
    way_begin = re.compile(r'\s*<way [^>]*>\s*')
    way_end = re.compile(r'\s*<\/way>\s*')
    _lat, _lon = next(iter(node_cache.values()))
    partition_ix, _ = next(filter(lambda p: is_in_partition(_lon, _lat, p[1]), enumerate(partitions)))

    node_cache = {key: (*value, partition_ix) for key, value in node_cache.items()}
    way_lines = None
    for _, line in parser.get_lines():
        if way_lines:
            way_lines.append(line)

        if way_begin.match(line):
            way_lines = [line]
            continue
        if way_end.match(line):
            way_lines.append(line)
            way_nodes, way_tags = OpenStreetMapParser.parse_way(way_lines)
            way_lines = None
            if way_tags.get('building'):
                # TODO: We don't consider buildings part of the input graph
                continue

            # TODO: Should we consider it a loop??
            for node1, node2 in zip(way_nodes, way_nodes[1:]):
                if node1 in node_cache and node_cache[node1][2] == partition_ix:
                    edge_info = get_edge_info(node1, node2, node_cache, parser, partitions, partition_ix)
                    yield manager__pb2.GraphPiece(
                        edges=edge_info
                    )


class ManagerServiceServicer(manager_pb2_grpc.ManagerServiceServicer):
    """Interface exported by the parser/manager node.
    """

    def __init__(self, partitions, parser):
        self.partitions = partitions
        self.parser = parser
        self.n_partitions = len(partitions)
        self.workers = {}
        self.workers_lock = threading.Lock()

    def RegisterWorker(self, request, context):
        """Methods for use by workers
        """
        with self.workers_lock:
            worker_addr = request.listening_address
            worker_id = self.workers.get(worker_addr, len(self.workers) + 1)
            self.workers[worker_id] = worker_addr
        return manager__pb2.WorkerMetadata(worker_id=worker_id)

    def GetGraphFragment(self, request, context):
        """Missing associated documentation comment in .proto file."""
        try:
            with self.workers_lock:
                partition_ix = request.worker_id - 1
            node_cache = self.parser.get_partition_nodes(self.partitions, partition_ix)
            for i, node_id in enumerate(node_cache):
                node_info = manager__pb2.Node(node_id=node_id)
                yield manager__pb2.GraphPiece(
                    nodes=node_info
                )
            for edge in send_partition_edges(node_cache, self.partitions.copy(), self.parser):
                yield edge
        except Exception as e:
            context.set_code(grpc.StatusCode.ABORTED)
            context.set_details(f"What: {e}, Traceback: {traceback.format_exc()}")
            # context.set_details(f"What: {e}")

    def GetWorkersList(self, request, context):
        """Methods for use by executers
        """
        with self.workers_lock:
            workers = [
                manager__pb2.WorkersList.WorkerEntry(worker_id=worker_id, address=address)
                for worker_id, address in self.workers.items()
            ]
        return manager__pb2.WorkersList(workers=workers)


def add_ManagerServiceServicer_to_server(servicer, server):
    rpc_method_handlers = {
            'RegisterWorker': grpc.unary_unary_rpc_method_handler(
                    servicer.RegisterWorker,
                    request_deserializer=google_dot_protobuf_dot_empty__pb2.Empty.FromString,
                    response_serializer=manager__pb2.WorkerMetadata.SerializeToString,
            ),
            'GetGraphFragment': grpc.unary_stream_rpc_method_handler(
                    servicer.GetGraphFragment,
                    request_deserializer=google_dot_protobuf_dot_empty__pb2.Empty.FromString,
                    response_serializer=manager__pb2.GraphPiece.SerializeToString,
            ),
            'GetWorkersList': grpc.unary_unary_rpc_method_handler(
                    servicer.GetWorkersList,
                    request_deserializer=google_dot_protobuf_dot_empty__pb2.Empty.FromString,
                    response_serializer=manager__pb2.WorkersList.SerializeToString,
            ),
    }
    generic_handler = grpc.method_handlers_generic_handler(
            'manager.ManagerService', rpc_method_handlers)
    server.add_generic_rpc_handlers((generic_handler,))


 # This class is part of an EXPERIMENTAL API.
class ManagerService(object):
    """Interface exported by the parser/manager node.
    """

    @staticmethod
    def RegisterWorker(request,
            target,
            options=(),
            channel_credentials=None,
            call_credentials=None,
            insecure=False,
            compression=None,
            wait_for_ready=None,
            timeout=None,
            metadata=None):
        return grpc.experimental.unary_unary(request, target, '/manager.ManagerService/RegisterWorker',
            google_dot_protobuf_dot_empty__pb2.Empty.SerializeToString,
            manager__pb2.WorkerMetadata.FromString,
            options, channel_credentials,
            insecure, call_credentials, compression, wait_for_ready, timeout, metadata)

    @staticmethod
    def GetGraphFragment(request,
            target,
            options=(),
            channel_credentials=None,
            call_credentials=None,
            insecure=False,
            compression=None,
            wait_for_ready=None,
            timeout=None,
            metadata=None):
        return grpc.experimental.unary_stream(request, target, '/manager.ManagerService/GetGraphFragment',
            google_dot_protobuf_dot_empty__pb2.Empty.SerializeToString,
            manager__pb2.GraphPiece.FromString,
            options, channel_credentials,
            insecure, call_credentials, compression, wait_for_ready, timeout, metadata)

    @staticmethod
    def GetWorkersList(request,
            target,
            options=(),
            channel_credentials=None,
            call_credentials=None,
            insecure=False,
            compression=None,
            wait_for_ready=None,
            timeout=None,
            metadata=None):
        return grpc.experimental.unary_unary(request, target, '/manager.ManagerService/GetWorkersList',
            google_dot_protobuf_dot_empty__pb2.Empty.SerializeToString,
            manager__pb2.WorkersList.FromString,
            options, channel_credentials,
            insecure, call_credentials, compression, wait_for_ready, timeout, metadata)
