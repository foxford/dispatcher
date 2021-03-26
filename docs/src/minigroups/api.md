# API

All routes expect json payloads.

### Routes
Route                                           | Method | Short description
----------------------------------------------- | ------ | ----------
/api/v1/minigroups/:minigroup_id                | GET    | [Reads](#read-minigroup) minigroup.
/api/v1/audiences/:audience/minigroups/:scope   | GET    | [Reads](#read-minigroup) minigroup.
/api/v1/minigroups                              | POST   | [Creates](#create-minigroup) minigroup and required rooms in other services.
/api/v1/minigroups/:minigroup_id                | PUT    | [Updates](#update-minigroup) minigroup.

### Create minigroup

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
time                   | [int, int]  | +        | Start and end
tags                   | json object | +        | Arbitrary tags.
host                   | string      |          | Host of the nightmare
reserve                | i32         | +        | Slots to reserve on janus backend.
locked_chat            | bool        | +        | Lock chat in created event room

Response: status 201 and minigroup object as payload.

### Read minigroup

Parameters either

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
minigroup_id             | uuid        |          | minigroup id

Or:

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | ------------------
audience             | string      |          | Minigroup audience
scope                | string      |          | Minigroup scope

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
id                     | string      |          | Minigroup scope
real_time              | json object | +        | `event_room_id`, `conference_room_id` and `host` fields

Response: status 200 and minigroup object as payload.

### Update minigroup

Parameters:

Свойство               | Тип         | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time

Response: status 200 and minigroup object as payload.
