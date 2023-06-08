# API

All routes expect json payloads.

1. Get last ban operation id.
2. Send it in POST request along with other data.
If there's an error, return to 1.

### Routes
Route                           | Method | Short description
------------------------------- | ------ | ----------
/api/v1/account/:account_id/ban | POST   | [Bans](#ban) media stream and collaboration for user
/api/v1/account/:account_id/ban | GET    | [Gets](#get-last-ban-operation) last ban operation id for user

### Ban

Path variable          | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
account_id             | string      |          | Account id we want to ban

JSON payload is required with following attributes:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
last_seen_op_id        | integer     |          | Last seen operation id which can be found by [GET request](#get-last-ban-operation)
ban                    | bool        |          | Ban/unban
class_id               | uuid        |          | User should be banned in this particular classroom 

Response: status 200 and empty payload or json object with an error description.

### Get last ban operation

Path variable          | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
account_id             | string      |          | Account id we want to ban

No JSON payload is expected.

Response: status 200 and following payload or json object with an error description.

Attribute       | Type    | Optional | Description
last_seen_op_id | integer |          | Last seen operation id which can be used later in POST request
