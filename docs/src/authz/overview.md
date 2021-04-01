# Authorization

In order to authorize an **action** performed by a **subject** to an **object**, the application sends a `POST` request to the authorization endpoint.

**Example**

```json
{
    "subject": {
        "namespace": "iam.example.org",
        "value": "123e4567-e89b-12d3-a456-426655440000"
    },
    "object": {
        "namespace": "dispatcher.svc.example.org",
        "value": ["webinars", "123e4567-e89b-12d3-a456-426655440000"]
    },
    "action": "read"
}
```

Subject's namespace and account label are retrieved from `audience` and `account_label` properties of MQTT message respectively or `Authorization` header `Bearer ${token}` token of HTTP request.

URI of authorization endpoint, object and anonymous namespaces are configured through the application configuration file.

Possible values for `OBJECT` and `ACTION` are listed under each class type:
* [chats](./chats.md)
* [classrooms](./classrooms.md)
* [webinars](./webinars.md)
