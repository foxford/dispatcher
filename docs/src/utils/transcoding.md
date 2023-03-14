# API

All routes expect json payloads.

### Routes
Route                                               | Method | Short description
--------------------------------------------------- | ------ | ----------
/api/v1/transcoding/minigroup/:minigroup_id/restart | POST   | [Restarts](#restart-tq-minigroup) transcoding of minigroup after room.adjust stage
/api/v1/transcoding/webinar/:webinar_id/restart     | POST   | [Restarts](#restart-tq-webinar) transcoding of webinar after room.adjust stage


### Restart TQ minigroup

Path variable          | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
minigroup_id           | uuid        |          | Minigroup id

JSON payload is optional with following attributes:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
priority               | string      |          | One of 'low', 'normal' or 'high'. 'normal' is default

Response: status 200 and empty payload or json object with an error description.


### Restart TQ webinar

Path variable          | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
webinar_id             | uuid        |          | Webinar id

JSON payload is optional with following attributes:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
priority               | string      |          | One of 'low', 'normal' or 'high'. 'normal' is default

Response: status 200 and empty payload or json object with an error description.
