# masterserve

A system built using this toolkit consists of three types of logical entities: master servers, game servers, and game
clients.

Every **game server** maintains a connection to a master server, sending an opaque heartbeat blob at regular
intervals. To implement a game server, connect to a master server, then pass the connection and a `Stream` of heartbeats
to `masterserve_heartbeat::run`.

**Game clients** may connect to a master server to receive the set of currently-connected game servers and the latest
heartbeat from each, updated at regular intervals. To implement a game client, connect to a master server, then pass the
connection to `masterserve_client::run`.

The **master server** stores the latest heartbeat from every currently-connected server. A complete implementation is
provided by the `daemon` crate.

All communications are performed over QUIC, using `quinn` connections. Downstream code is responsible for
establishing connections and recovering from connection loss if necessary; this ensures that arbitrary connection
configurations can be used.
