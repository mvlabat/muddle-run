# Muddle Run

[![Good first issues](https://img.shields.io/github/issues/mvlabat/muddle-run/good%20first%20issue?label=good%20first%20issues&color=7057ff)](https://github.com/mvlabat/muddle-run/issues)
[![Help wanted](https://img.shields.io/github/issues/mvlabat/muddle-run/help%20wanted?label=help%20wanted&color=008672)](https://github.com/mvlabat/muddle-run/issues)
[![CI](https://github.com/mvlabat/muddle-run/workflows/CI/badge.svg)](https://github.com/mvlabat/muddle-run/actions)

A home for experiments for [muddle.run](http://muddle.run).

https://user-images.githubusercontent.com/2943388/125176134-cb4e6f00-e1d9-11eb-8fc8-6d9aa5c09583.mp4

## Project state

Currently, the project represents the very bare-bones of the game I'm trying to build:
a multiplayer runner with a collaborative level editor, which will also allow players
to test levels while they are being designed.

### Current features
- WASD movement
- [Rapier](https://github.com/dimforge/bevy_rapier) physics
- Netcode (poorly executed one, but inspired by [Overwatch's GDC presentation](https://youtu.be/W3aieHjyNvw))
  - Interpolation
  - Rewinding game state
  - Real-time adjustment of client simulation speed
- Initial version of Builder mode (collaborative level editor)

### Roadmap
- [Gameplay](https://github.com/mvlabat/muddle-run/projects/6)
- [Builder Mode](https://github.com/mvlabat/muddle-run/projects/2)
- [Techdebt, Performance & Stability](https://github.com/mvlabat/muddle-run/projects/5)

## Building and running

This application can be run in either UPD, or WebRTC mode.
The workspace contains the following binary projects:

- `mr_dekstop_client`
- `mr_web_client`
- `mr_server` 
- `mr_matchmaker`
- `mr_persistence`

### Running the desktop client and the server

```bash
# Running the server
# (Note that 127.0.0.1 might not work for Windows, you can use your local network ip instead, like 192.168.x.x)
# (See https://github.com/naia-rs/naia-socket/issues/24)
MUDDLE_PUBLIC_IP_ADDR=127.0.0.1 MUDDLE_LISTEN_PORT=3455 cargo run -p mr_server

# Running the client
cargo run -p mr_desktop_client
```

### Running the web client and the server

```bash
# Running the server
# (Note that 127.0.0.1 might not work for Firefox, you can use your local network ip instead, like 192.168.x.x)
MUDDLE_PUBLIC_IP_ADDR=127.0.0.1 MUDDLE_LISTEN_PORT=3455 cargo run -p mr_server

# Running the client
cd bins/web_client
wasm-pack build --target web
basic-http-server . # or any other tool that can serve static files
```

### Running the persistence service

#### Prerequisites:

- PostgreSQL database
- sqlx-cli
- Auth0 and Google OAuth 2.0 client credentials (see the environment variables section)

#### Creating a database

```
DATABASE_URL=postgres://postgres@localhost/mr_persistence_development sqlx database setup
```

### Environment variables

Environment variables are read when both compiling the binaries and running
them (except for the web client). The environment variables that are read
during the run-time take higher priority.

Dotenv files are also supported. They are recursively searched down to the
root starting from a working directory. The binaries search for the following
files:

- .env
- .env.$MUDDLE_ENV
- .env.$CARGO_PKG_NAME
- .env.$CARGO_PKG_NAME.$MUDDLE_ENV

Every found file is loaded, and all read variables are merged. Priority is
defined as in the list above, where the most specific dotenv file overrides
the vars from the other ones.

`MUDDLE_ENV` equals `production` when compiling with the release profile
and `development` when compiling with the debug one. It can be overridden
with an environment variable.

#### `mr_server`

- `MUDDLE_PUBLIC_IP_ADDR` (mandatory if outside Agones cluster)
  - It can't equal to `0.0.0.0`, use `127.0.0.1` if you want to connect to localhost, for instance.
  - Also, note that `127.0.0.1` might not work for Firefox, you can use your local network instead, like `192.168.x.x`.
- `MUDDLE_LISTEN_IP_ADDR` (defaults to `0.0.0.0`)
- `MUDDLE_LISTEN_PORT` (mandatory if outside Agones cluster)
- `MUDDLE_IDLE_TIMEOUT` (defaults to 300)
  - Specifies the time in milliseconds after which a server will be closed if there are no connected players.
- `MUDDLE_PUBLIC_PERSISTENCE_URL` (optional)
- `MUDDLE_PRIVATE_PERSISTENCE_URL` (optional)
- `MUDDLE_GOOGLE_WEB_CLIENT_ID` (mandatory if persistence urls are set)
- `MUDDLE_GOOGLE_DESKTOP_CLIENT_ID` (mandatory if persistence urls are set)
- `MUDDLE_AUTH0_CLIENT_ID` (mandatory if persistence urls are set)

#### `mr_desktop_client` and `mr_web_client`

- `MUDDLE_SERVER_IP_ADDR` (defaults to `127.0.0.1`)
- `MUDDLE_SERVER_PORT` (defaults to `3455`)
- `MUDDLE_MATCHMAKER_URL` (optional)
  - If this variable is passed, `MUDDLE_SERVER_IP_ADDR` and `MUDDLE_SERVER_PORT` are still read,
  but the server is displayed in the list only if `MUDDLE_SERVER_IP_ADDR` is passed explicitly
  (the default value is ignored).
- `MUDDLE_PUBLIC_PERSISTENCE_URL` (mandatory if `MUDDLE_MATCHMAKER_URL` is set)
- `MUDDLE_GOOGLE_CLIENT_ID` (mandatory if `MUDDLE_MATCHMAKER_URL` is set)
- `MUDDLE_GOOGLE_CLIENT_SECRET` (mandatory for the desktop client if `MUDDLE_MATCHMAKER_URL` is set)
  - Google only considers Desktop clients as public ones, also requiring client secrets in the requests.
  **Don't set the secret for the web clients.**
- `MUDDLE_AUTH0_CLIENT_ID` (mandatory if `MUDDLE_MATCHMAKER_URL` is set)

#### Both client and server

- `SIMULATIONS_PER_SECOND` (defaults to `120`, **compile-time only**)
  - Is expected to work with the following values: `30`, `60`, `120`. You may want to set a lower value than the
    default one if your device can't handle 120 simulations per second.
  - **Note** that both the server and the client
    must be compiled with the same value.

#### `mr_persistence`

- `MUDDLE_GOOGLE_WEB_CLIENT_ID` (mandatory)
- `MUDDLE_GOOGLE_DESKTOP_CLIENT_ID` (mandatory)
- `MUDDLE_AUTH0_CLIENT_ID` (mandatory)

## Building docker images

### mr_matchmaker

```bash
docker build -t mvlabat/mr_matchmaker -f mr_matchmaker.dockerfile . --platform linux/amd64
```

### mr_web_client

```bash
docker build -t mvlabat/mr_web_client --build-arg muddle_matchmaker_ip_addr=<IP> --build-arg muddle_matchmaker_port=<PORT> -f mr_web_client.dockerfile .  --platform linux/amd64
```

### mr_server

```bash
docker build -t mvlabat/mr_server -f mr_server.dockerfile . --platform linux/amd64
```

## DevOps

### Prerequisites

- [aws-cli](https://aws.amazon.com/cli/) (tested with 2.3.2)
  - Make sure to [configure AWS CLI](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-configure.html): `aws configure` 
- [kubectl](https://kubernetes.io/docs/tasks/tools/) (v1.21.0)
- [helm](https://helm.sh/docs/intro/install/) (tested with v3.7.1)

### Applying

1. `terraform apply -target=module.eks_cluster`
2. `aws eks --region <region-code> update-kubeconfig --name <cluster_name>`
3. `terraform apply -target=module.helm_agones`
   - Resources declared with `kubernetes_manifest` fail to plan without this helm release installed first
4. `terraform apply`

### Destroying

- `terraform destroy -target=module.agones`
- `helm delete agones -n agones-system && terraform destroy -target=module.helm_agones`
- `terraform destroy -target=module.eks_cluster`

### Updating deployment

```bash
kubectl set image deployment <DEPLOYMENT_NAME> <CONTAINER_NAME>=<TAG>
```

For example:

- `kubectl set image deployment mr-matchmaker mr-matchmaker=mvlabat/mr_matchmaker`
  - You can also alternate appending and removing `:latest` tag suffix, to trick kubernetes into redeploying
    (otherwise it might think that the image is the same and won't pull its updated version).

### Troubleshooting

- **Error: Kubernetes cluster unreachable: invalid configuration: no configuration has been provided, try setting KUBERNETES_MASTER environment variable**
  
  To fix this error, run `export KUBE_CONFIG_PATH=~/.kube/config` (or add it to your shell rc).

  - Also, make sure you ran `aws eks --region <region-code> update-kubeconfig --name <cluster_name>` before.
  