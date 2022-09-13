# API

All routes expect json payloads.

### Routes
Route                                           | Method | Short description
----------------------------------------------- | ------ | ----------
/api/v1/webinars/:webinar_id                    | GET    | [Reads](#read-webinar) webinar.
/api/v1/audiences/:audience/webinars/:scope     | GET    | [Reads](#read-webinar) webinar.
/api/v1/webinars                                | POST   | [Creates](#create-webinar) webinar and required rooms in other services.
/api/v1/webinars/:webinar_id/replicas           | POST   | [Creates](#create-webinar-replica) a replica of webinar and a room in the `conference` service (the `event` room is taken from the original webinar).
/api/v1/webinars/:webinar_id                    | PUT    | [Updates](#update-webinar) webinar.
/api/v1/webinars/convert                        | POST   | [Creates](#convert-webinar) webinar with already existing event and conference rooms.
/api/v1/webinars/:webinar_id/download           | GET    | [Downloads](#download-webinar) webinar source file.
/api/v1/webinars/:webinar_id/recreate           | POST   | [Recreates](#recreate-webinar) webinar rooms.
/api/v1/webinars/:webinar_id/events             | POST   | [Creates](#create-webinar-event) event in the room.
/api/v1/webinars/:webinar_id/timestamps         | POST   | [Records](#save-position) current position while viewing a recording.
/api/v1/webinars/:id/properties/:property_id    | GET    | [Reads](#read-property) the property
/api/v1/webinars/:id/properties/:property_id    | PUT    | [Updates](#update-property) the property


### Create webinar

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
time                   | [int, int]  | +        | Start and end
tags                   | json object | +        | Arbitrary tags.
properties             | json object | +        | Arbitrary class properties.
reserve                | i32         | +        | Slots to reserve on janus backend.
locked_chat            | bool        | +        | Lock chat in created event room (defaults to true)

Response: status 201 and webinar object as payload.

### Create webinar replica

Request parameters:

| Attribute         | Type   | Optional | Description |
|-------------------|--------|----------|-------------|
| original_class_id | uuid   |          | Webinar id  |
| scope             | string |          | Scope       |
| audience          | string |          | Audience    |

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

Query parameter        | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
class_keys             | [string]    | +        | List of webinar properties to fetch
account_keys           | [string]    | +        | List of account properties to fetch

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
class_id               | uuid        |          | Webinar id (or original webinar id, if it exists)
id                     | string      |          | Webinar scope
real_time              | json object | +        | `event_room_id` and `conference_room_id` fields
on_demand              | json array  | +        | Array with original and modified stream versions. Modified stream contains `room_events_uri` with s3 link to dumped events.
status                 | string      | +        | Webinar state, possible values: `transcoded`, `adjusted`, `finished`, `real-time`, `closed`
position               | int         | +        | Previously saved viewership position
turn_host              | string      | +        | TURN host to connect to if needed
content_id             | string      |          | Webinar id or scope

Response: status 200 and webinar object as payload.

### Update webinar

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
time                   | [int, int]  | +        | New time
reserve                | int         | +        | New reserve

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
properties             | json object | +        | Arbitrary class properties
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

Parameters

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
locked_chat            | bool        | +        | Lock chat in created event room (defaults to true)

Response: status 200 and webinar object as payload.


### Create webinar event

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

### Save position

Parameters:

Name          | Type    | Default    | Description
------------- | ------- | ---------- | -----------------------------
position      | int     | _required_ | Position to save (in seconds)

Response: status **201** and empty payload.


### Read property

Route parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
id                     | uuid        |          | Webinar id
property_id            | string      |          | Property id is any string

Response: status 200 and requested property as payload.


### Update property

Route parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
id                     | uuid        |          | Webinar id
property_id            | string      |          | Property id is any string

Request body:

Any valid JSON value that should be associated with the given property id.

Response: status 200 and updated class properties as payload.
