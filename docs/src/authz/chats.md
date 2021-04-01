# Chats authorization objects

Object                       | Action  | Description
---------------------------- | ------- | ------------
["chats"]                                                  | create  | Tenant attempts to [create](/chats/api.md#create-chat) a chat
["chats"]                                                  | convert | Tenant attempts to [convert](/chats/api.md#convert-chat) already existings room into a chat
["chats", CHAT_ID]                                         | read    | User reads chat state
["chats", CHAT_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new event [^1] in the chat
["chats", CHAT_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new claim [^1] in the chat
["chats", CHAT_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create  | User alter an event [^1] somehow

[^1]: Types, claims and attributes are documented [separately](./events.md)
