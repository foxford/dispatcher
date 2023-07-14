# Scopes dispatching

Dispatcher service accepts connections at some http route and redirects to different routes based on request params.
Expected to serve different frontends based on different scopes.

## Default url

Is constructed from `frontend.{:tenant}.base_url` by replacing its host with `{:tenant}.{:app}.{:frontend.{:tenant}.base_url}`

### Routes

| Path                           | Method | Description                                                            |
|--------------------------------|--------|------------------------------------------------------------------------|
| /info/scopes                   | GET    | List of all scopes                                                     |
| /info/frontends                | GET    | List of all frontends.                                                 |
| /api/v1/redirs                 | GET    | Redirects either to frontend found by scope and app or to default url. |
| /api/v1/scopes/:scope/rollback | POST   | Deletes the scope.                                                     |
| /api/v1/healthz                | GET    | Responds `Ok`                                                          |
