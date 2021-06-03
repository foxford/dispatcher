# API

All routes expect json payloads.

### Routes
Route                                     | Method | Short description
----------------------------------------- | ------ | ----------
/api/v1/chats/:chat_id                    | GET    | [Reads](#read-chat) chat.
/api/v1/audiences/:audience/chats/:scope  | GET    | [Reads](#read-chat) chat.
/api/v1/chats                             | POST   | [Creates](#create-chat) chat and corresponding room in event.
/api/v1/chats/convert                     | POST   | [Creates](#convert-chat) chat with already existing event room.
/api/v1/chats/:chat_id/events             | POST   | [Creates](#create-chat-event) event in the room.

### Create chat

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags.

Response: status 201 and chat object as payload.

### Read chat

Parameters either

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | --------------
chat_id              | uuid        |          | Chat id

Or:

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | ------------------
audience             | string      |          | Chat audience
scope                | string      |          | Chat scope

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
id                     | string      |          | Chat scope
real_time              | json object | +        | `event_room_id` field

Response: status 200 and chat object as payload.

### Convert chat

A tenant may wish to create a chat with event room already created earlier. It can use this method.

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags
event_room_id          | uuid        |          | Event room uuid

Response: status 201 and chat object as payload.

### Create chat event

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
