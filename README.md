# masterserve

Multiplayer games often want to present players with a list of servers to choose from, with information about each
server's current state such as server name, selected map, and player count. To support this, masterserve provides a
toolkit for aggregating small "heartbeat" blobs of information from game servers and distributing the aggregated data to
clients on demand, labeling each blob with the address of the originating game server. Masterserve does not enforce any
structure within an individual heartbeat blob; it's up to calling code to decide what information game servers will
publish and how it is encoded.

A system built using this toolkit consists of three types of logical entities: master servers, game servers, and game
clients.

Every **game server** maintains a connection to a master server, sending a heartbeat blob at regular intervals. To
implement a game server, connect to a master server, then pass the connection and a `Stream` of heartbeats to
`masterserve_heartbeat::run`.

**Game clients** may connect to a master server to receive the set of currently-connected game servers and the latest
heartbeat from each, updated at regular intervals. Game servers are identified to clients by the IP address and port
they contacted the master server from. To implement a game client, connect to a master server, then pass the connection
to `masterserve_client::run`.

The **master server** stores the latest heartbeat from every currently-connected server. A complete implementation is
provided in `daemon`.

All communications are performed over QUIC, using `quinn` connections. Downstream code is responsible for
establishing connections and recovering from connection loss if necessary; this ensures that arbitrary connection
configurations can be used.
