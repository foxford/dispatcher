# Proxy

Dispatcher performs proxying of authz requests.

If authz object starts with `["room", ROOM_ID, ..]` and dispatcher manages to find a webinar/classroom/chat corresponding to the room,
the authz object will be altered.

Otherwise, the request will be proxied as is (only if the audience param is a valid audience for authorization)

The rules for this modification are[^1]:

* if request comes from `event`:

Object                          | Action      | New object      | New action
------------------------------- | ----------- | --------------- | ------------
["rooms", ROOM_ID, "agents"]    | `list`      | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, "events"]    | `list`      | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, "events"]    | `subscribe` | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, ..]          | *           | [TYPE, ROOM_ID] | no change[^2]
\*                              | *           | no change       | no change[^3]

* if request comes from `conference`:

Object                          | Action      | New object      | New action
------------------------------- | ----------- | --------------- | ------------
["rooms", ROOM_ID, "agents"]    | `list`      | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, "rtcs"]      | `list`      | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, "events"]    | `subscribe` | [TYPE, ROOM_ID] | `read`
["rooms", ROOM_ID, ..]          | *           | [TYPE, ROOM_ID] | no change[^2]
\                               | *           | no change       | no change[^3]

[^1]: `["rooms", ROOM_ID, ..]` means an array containing at least 2 elements, you can read `, ..` as "0 or more elements". `TYPE` is placeholder for `"webinars"`, `"classrooms"`, `"chats"` etc, depending on results of search by room id.

[^2]: If a class corresponding to the room is found action is left as is, only object is altered.

[^3]: if a class corresponding to the room is not found everything stays the same.
