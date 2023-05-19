# Scopes dispatching

Dispatcher service accepts connections at some http route and redirects to different routes based on request params.
Expected to serve different frontends based on different scopes.

## Default url

Is constructed from `default_frontend_base` by replacing its host with `{:tenant}.{:app}.{:default_frontend_base.host}`

### Routes
Path                                  | Method  | Description
------------------------------------- | ------- | ------------------
/api/v1/redirs                        | GET     | Redirects either to frontend found by scope and app or to default url.
/api/v1/scopes/:scope/rollback        | POST    | Deletes the scope.
/api/v1/healthz                       | GET     | Responds `Ok`
