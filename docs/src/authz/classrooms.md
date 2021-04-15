# Classrooms authorization objects

Object                                                               | Action  | Description
-------------------------------------------------------------------- | ------- | ------------
["classrooms"]                                                       | create  | Tenant [creates](/classrooms/api.md#create-classroom) a classroom
["classrooms"]                                                       | convert | Tenant [converts](/classrooms/api.md#update-classroom) already existings rooms into a classroom
["classrooms", CLASSROOM_ID]                                         | read    | User [reads](/classrooms/api.md#read-classroom) the classroom state
["classrooms", CLASSROOM_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new event [^1] in the classroom
["classrooms", CLASSROOM_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new claim [^1] in the classroom
["classrooms", CLASSROOM_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create  | User alter an event [^1] somehow
["classrooms", CLASSROOM_ID, "sets", "content"]                      | read    | User wants to read a file from storage
["classrooms", CLASSROOM_ID, "sets", "content"]                      | create  | User wants to upload a file to storage
["classrooms", CLASSROOM_ID, "sets", "content"]                      | delete  | User wants to delete a file from storage

[^1]: Types, claims and attributes are documented [separately](./events.md)
