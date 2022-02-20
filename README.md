# metaserve

Multiplayer games often want to present players with a list of servers to choose from, with information about each
server's current state such as server name, selected map, and player count. To support this, metaserve provides a
toolkit for aggregating small "heartbeat" blobs of information from game servers and distributing the aggregated data to
clients on demand, labeling each blob with the address of the originating game server. Metaserve does not enforce any
structure within an individual heartbeat blob; it's up to calling code to decide what information game servers will
publish and how it is encoded.

A system built using this toolkit consists of three types of logical entities: meta servers, game servers, and game
clients.

Every **game server** maintains a connection to a meta server, sending a heartbeat blob at regular intervals. To
implement a game server, connect to a meta server, then pass the connection and a `Stream` of heartbeats to
`metaserve_heartbeat::run`.

**Game clients** may connect to a meta server to receive the set of currently-connected game servers and the latest
heartbeat from each, updated at regular intervals. Game servers are identified to clients by the IP address and port
they contacted the meta server from. To implement a game client, connect to a meta server, then pass the connection
to `metaserve_client::run`.

The **meta server** stores the latest heartbeat from every currently-connected server. A complete implementation is
provided in `daemon`.

All communications are performed over QUIC, using `quinn` connections. Downstream code is responsible for
establishing connections and recovering from connection loss if necessary; this ensures that arbitrary connection
configurations can be used.
