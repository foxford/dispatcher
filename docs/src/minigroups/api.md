# API

All routes expect json payloads.

### Routes
Route                                           | Method | Short description
----------------------------------------------- | ------ | ----------
/api/v1/minigroups/:minigroup_id                | GET    | [Reads](#read-minigroup) minigroup.
/api/v1/audiences/:audience/minigroups/:scope   | GET    | [Reads](#read-minigroup) minigroup.
/api/v1/audiences/:audience/minigroups/:scope   | PUT    | [Updates](#update-minigroup) minigroup.
/api/v1/minigroups                              | POST   | [Creates](#create-minigroup) minigroup and required rooms in other services.
/api/v1/minigroups/:minigroup_id                | PUT    | [Updates](#update-minigroup) minigroup.
/api/v1/minigroups/:minigroup_id/download       | GET    | [Downloads](#download-minigroup) minigroup source file.
/api/v1/minigroups/:minigroup_id/events         | POST   | [Creates](#create-minigroup-event) event in the room.
/api/v1/minigroups/:minigroup_id/recreate       | POST   | [Recreates](#recreate-minigroup) minigroup rooms.

### Create minigroup

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
time                   | [int, int]  | +        | Start and end
tags                   | json object | +        | Arbitrary tags.
host                   | string      |          | Host account id
reserve                | i32         | +        | Slots to reserve on janus backend.
locked_chat            | bool        | +        | Lock chat in created event room (defaults to true)

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
All parameters are optional but at least one is expected

Свойство               | Тип         | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time
reserve                | int         | +        | New reserve
host                   | string      | +        | Host agent id

Response: status 200 and minigroup object as payload.

### Create minigroup event

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

### Recreate minigroup

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time
locked_chat            | bool        | +        | Lock chat in created event room (defaults to true)

Response: status 200 and minigroup object as payload.

### Download minigroup

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
minigroup_id           | uuid        |          | Minigroup id

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
url
