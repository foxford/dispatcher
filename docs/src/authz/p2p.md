# P2P authorization objects

Object                                                         | Action  | Description
-------------------------------------------------------------- | ------- | ------------
["classrooms"]                                                 | create  | Tenant [creates](/p2p/api.md#create-p2p) a p2p
["classrooms"]                                                 | convert | Tenant [converts](/p2p/api.md#update-p2p) already existings rooms into a classroom
["classrooms", P2P_ID]                                         | read    | User [reads](/p2p/api.md#read-p2p) the p2p state
["classrooms", P2P_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new event [^1] in the p2p
["classrooms", P2P_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new claim [^1] in the p2p
["classrooms", P2P_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create  | User alter an event [^1] somehow
["classrooms", P2P_ID, "content"]                              | update  | User uploads or deletes some content (pictures, pdfs etc)

[^1]: Types, claims and attributes are documented [separately](./events.md)
