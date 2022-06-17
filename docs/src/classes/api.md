# Classes API

All routes expect json payloads.

### Routes
Route                                                       | Method | Short description
----------------------------------------------------------- | ------ | ----------
/api/v1/audiences/:audience/classes/:scope/editions/:id     | POST   | Commits edition with id=:id of a class with scope=:scope
/api/v1/classes/:class_id/properties/:property_id           | GET    | [Reads](#read-property) the property
/api/v1/classes/:class_id/properties/:property_id           | PUT    | [Updates](#update-property) the property

### Read property

Route parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
class_id               | uuid        |          | Class id
property_id            | string      |          | Property id is any string

Response: status 200 and requested property as payload.


### Update property

Route parameters:

Attribute              | Type        | Optional | Description
---------------------- | ----------- | -------- | -------------------------------------------------
class_id               | uuid        |          | Class id
property_id            | string      |          | Property id is any string

Request body:

Any valid JSON value that should be associated with the given property id.

Response: status 200 and updated class properties as payload.
