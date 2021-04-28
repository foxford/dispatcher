# Chats authorization objects

Object                       | Action  | Description
---------------------------- | ------- | ------------
["classrooms"]                                                  | create  | Tenant attempts to [create](/chats/api.md#create-chat) a chat
["classrooms"]                                                  | convert | Tenant attempts to [convert](/chats/api.md#convert-chat) already existings room into a chat
["classrooms", CHAT_ID]                                         | read    | User reads chat state
["classrooms", CHAT_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new event [^1] in the chat
["classrooms", CHAT_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new claim [^1] in the chat
["classrooms", CHAT_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create  | User alter an event [^1] somehow

[^1]: Types, claims and attributes are documented [separately](./events.md)
