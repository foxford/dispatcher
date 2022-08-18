# Proxy

Dispatcher performs proxying of authz requests.

The request will be proxied as is (only if the audience param is a valid audience for authorization)

The rules for this modification are:

* if request comes from `storage (v2)`:

Object                          | Action      | New object                                      | New action
------------------------------- | ----------- | ----------------------------------------------- | ------------
["sets", "origin" <> _]         | *           | ["classrooms", ID]                              | upload
["sets", "ms" <> _]             | *           | ["classrooms", ID]                              | download
["sets", "meta" <> _]           | read        | ["classrooms", ID]                              | read
["sets", "hls" <> _]            | read        | ["classrooms", ID]                              | read
["sets", "content" <> _]        | create      | ["classrooms", ID, content]                     | update
["sets", "content" <> _]        | delete      | ["classrooms", ID, content]                     | update
["sets", "content" <> _]        | read        | ["classrooms", ID]                              | read
["sets", SET]                   | *           | ["classrooms", ID, "sets", BUCKET_PREFIX][^4]   | no change[^2]
\*                              | *           | no change                                       | no change[^3]

[^2]: If a class corresponding to the room is found action is left as is, only object is altered.

[^3]: if a class corresponding to the room or set is not found everything stays the same.

[^4]: set has format like `#{bucket_prefix.audience}::#{set_id}`
