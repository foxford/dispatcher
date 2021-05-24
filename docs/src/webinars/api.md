# API

All routes expect json payloads.

### Routes
Route                                           | Method | Short description
----------------------------------------------- | ------ | ----------
/api/v1/webinars/:webinar_id                    | GET    | [Reads](#read-webinar) webinar.
/api/v1/audiences/:audience/webinars/:scope     | GET    | [Reads](#read-webinar) webinar.
/api/v1/webinars                                | POST   | [Creates](#create-webinar) webinar and required rooms in other services.
/api/v1/webinars/:webinar_id                    | PUT    | [Updates](#update-webinar) webinar.
/api/v1/webinars/convert                        | POST   | [Creates](#convert-webinar) webinar with already existing event and conference rooms.
/api/v1/webinars/:webinar_id/download           | GET    | [Downloads](#download-webinar) webinar source file.
/api/v1/webinars/:webinar_id/recreate           | POST   | [Recreates](#move-webinar) webinar rooms.

### Create webinar

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
time                   | [int, int]  | +        | Start and end
tags                   | json object | +        | Arbitrary tags.
reserve                | i32         | +        | Slots to reserve on janus backend.
locked_chat            | bool        | +        | Lock chat in created event room

Response: status 201 and webinar object as payload.

### Read webinar

Parameters either

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
webinar_id             | uuid        |          | Webinar id

Or:

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | ------------------
audience             | string      |          | Webinar audience
scope                | string      |          | Webinar scope

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
id                     | string      |          | Webinar scope
real_time              | json object | +        | `event_room_id` and `conference_room_id` fields
on_demand              | json array  | +        | Array with original and modified stream versions.
status                 | string      | +        | Webinar state, possible values: `transcoded`, `adjusted`, `finished`, `realtime`

Response: status 200 and webinar object as payload.

### Update webinar

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time

Response: status 200 and webinar object as payload.

### Convert webinar

A tenant may wish to create a webinar with event and conference rooms already created earlier. It can use this method.

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
time                   | [int, int]  | +        | Start and end
tags                   | json object | +        | Arbitrary tags
conference_room_id     | uuid        |          | Conference room uuid
event_room_id          | uuid        |          | Event room uuid
original_event_room_id | uuid        | +        | Original event room id
modified_event_room_id | uuid        | +        | Modified event room id
recording              | recording   | +        | Recording object if recording exists

Recording:

Attribute              | Type         | Optional | Description
---------------------- | ------------ | -------- | -------------------------------------------------
stream_id              | uuid         |          | Stream id
segments               | [[int, int]] |          | Segments
modified_segments      | [[int, int]] |          | Modified segments
uri                    | string       |          | Recording uri

Response: status 201 and webinar object as payload.

### Download webinar

Parameters either

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
webinar_id             | uuid        |          | Webinar id

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
url                    | string      |          | Url, supplied with `access_token` this will let someone access the recording.


### Recreate webinar

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time

Response: status 200 and webinar object as payload.
