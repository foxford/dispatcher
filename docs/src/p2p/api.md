# API

All routes expect json payloads.

### Routes
Route                                   | Method | Short description
--------------------------------------- | ------ | ----------
/api/v1/p2p/:p2p_id                     | GET    | [Reads](#read-p2p) p2p.
/api/v1/audiences/:audience/p2p/:scope  | GET    | [Reads](#read-p2p) p2p.
/api/v1/p2p                             | POST   | [Creates](#create-p2p) p2p and required rooms in other services.
/api/v1/p2p/convert                     | POST   | [Creates](#convert-p2p) p2p with already existing event and conference rooms.
/api/v1/p2p/:p2p_id/events              | POST   | [Creates](#create-p2p-event) event in the room.

### Create p2p

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags.
whiteboard             | bool        | +        | Flag to add whiteboard to created event room (defaults to true)

Response: status 201 and p2p object as payload.

### Read p2p

Parameters either

Attribute      | Type        | Optional | Description
-------------- | ----------- | -------- | --------------
p2p_id         | uuid        |          | p2p id

Or:

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | ------------------
audience             | string      |          | P2p audience
scope                | string      |          | P2p scope

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
class_id               | uuid        |          | P2P id
id                     | string      |          | P2p scope
real_time              | json object | +        | `event_room_id` and `conference_room_id` fields

Response: status 200 and p2p object as payload.

### Convert p2p

A tenant may wish to create a p2p with event and conference rooms already created earlier. It can use this method.

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags
conference_room_id     | uuid        |          | Conference room uuid
event_room_id          | uuid        |          | Event room uuid

Response: status 201 and p2p object as payload.

### Create p2p event

Parameters:

Name          | Type    | Default    | Description
------------- | ------- | ---------- | -----------------------------
type          | string  | _required_ | The event type.
set           | string  |       type | Collection set's name.
label         | string  | _optional_ | Collection item's label.
attribute     | string  | _optional_ | An attribute for authorization and filtering.
data          | json    | _required_ | The event JSON payload.
is_claim      | boolean |      false | Whether to notify the tenant.
is_persistent | boolean |       true | Whether to persist the event.

Response: status **201** and empty payload.
