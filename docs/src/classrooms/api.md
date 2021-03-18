# API

All routes expect json payloads.

### Routes
Route                                          | Method | Short description
---------------------------------------------- | ------ | ----------
/api/v1/audiences/:audience/classrooms/:scope  | GET    | [Reads](#read-classroom) classroom.
/api/v1/classrooms                             | POST   | [Creates](#create-classroom) classroom and required rooms in other services.
/api/v1/classrooms/convert                     | POST   | [Creates](#convert-classroom) classroom with already existing event and conference rooms.

### Create classroom

Request parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags.

Response: status 201 and classroom object as payload.

### Read classroom

Attribute            | Type        | Optional | Description
-------------------- | ----------- | -------- | ------------------
audience             | string      |          | Classroom audience
scope                | string      |          | Classroom scope

Response:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | ---------------------------------------------------------
id                     | string      |          | Classroom scope
real_time              | json object | +        | `event_room_id` and `conference_room_id` fields

Response: status 200 and classroom object as payload.

### Convert classroom

A tenant may wish to create a classroom with event and conference rooms already created earlier. It can use this method.

Parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
audience               | string      |          | Audience
tags                   | json object | +        | Arbitrary tags
conference_room_id     | uuid        |          | Conference room uuid
event_room_id          | uuid        |          | Event room uuid

Response: status 201 and classroom object as payload.
