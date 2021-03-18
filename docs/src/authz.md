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

Possible values for `OBJECT` and `ACTION` are:

* for webinars

Object                            | Action  | Description
--------------------------------- | ------- | ------------
["webinars"]                      | create  | Tenant attempts to [create](webinars/api.md#create-webinar) a webinar
["webinars"]                      | convert | Tenant attempts to [convert](webinars/api.md#update-webinar) already existings rooms into a webinar
["webinars", WEBINAR_ID]          | update  | Tenant attempts to [update](webinars/api.md#update-webinar) a webinar
["webinars", WEBINAR_ID]          | read    | Agent attempts to [read](webinars/api.md#read-webinar) the webinar state


* for classrooms

Object                            | Action  | Description
--------------------------------- | ------- | ------------
["classrooms"]                    | create  | Tenant attempts to [create](classrooms/api.md#create-classroom) a classroom
["classrooms"]                    | convert | Tenant attempts to [convert](classrooms/api.md#update-classroom) already existings rooms into a classroom
["classrooms", CLASSROOM_ID]      | read    | Agent attempts to [read](classrooms/api.md#read-classroom) the classroom state
