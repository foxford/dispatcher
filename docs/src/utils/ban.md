# API

All routes expect json payloads.

### Routes
Route                           | Method | Short description
------------------------------- | ------ | ----------
/api/v1/account/:account_id/ban | POST   | [Bans](#ban) media stream and collaboration for user

### Ban

Path variable          | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
account_id             | string      |          | Account id we want to ban

JSON payload is required with following attributes:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | --------------
last_seen_op_id        | integer     |          | Last seen operation id which can be found in account info
ban                    | bool        |          | Ban/unban
class_id               | uuid        |          | User should be banned in this particular classroom 

Response: status 200 and empty payload or json object with an error description.
