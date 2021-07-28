### minigroup.ready

Arrives when stream postprocessing finishes.

Topic: `audience/:audience/events`

Attribute              | Type        | Optional | Описание
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
tags                   | json object | +        | Arbitrary tags
status                 | string      |          | "success"
id                     | uuid        |          | Minigroup id
stream_uri             | string      |          | S3 stream url
stream_id              | uuid        |          | Stream id
stream_duration        | u64         |          | Stream duration in seconds


### minigroup.stop

Arrives when minigroup ends.

Topic: `audience/:audience/events`

Attribute              | Type        | Optional | Описание
---------------------- | ----------- | -------- | -------------------------------------------------
scope                  | string      |          | Scope
tags                   | json object | +        | Arbitrary tags
id                     | uuid        |          | Minigroup id
