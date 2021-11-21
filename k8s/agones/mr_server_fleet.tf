resource "kubernetes_manifest" "mr_server_fleet" {
  manifest = {
    # Reference: https://agones.dev/site/docs/reference/fleet/
    apiVersion = "agones.dev/v1"
    kind       = "Fleet"
    # Fleet Metadata
    # https://v1-20.docs.kubernetes.io/docs/reference/generated/kubernetes-api/v1.20/#objectmeta-v1-meta
    metadata = {
      name      = "mr-server"
      namespace = "default"
    }
    spec = {
      # the number of GameServers to keep Ready or Allocated in this Fleet
      replicas = 0
      # defines how GameServers are organised across the cluster.
      # Options include:
      # "Packed" (default) is aimed at dynamic Kubernetes clusters, such as cloud providers, wherein we want to bin pack
      # resources
      # "Distributed" is aimed at static Kubernetes clusters, wherein we want to distribute resources across the entire
      # cluster
      scheduling = "Packed"
      # a GameServer template - see:
      # https://agones.dev/site/docs/reference/gameserver/ for all the options
      strategy = {
        # The replacement strategy for when the GameServer template is changed. Default option is "RollingUpdate",
        # "RollingUpdate" will increment by maxSurge value on each iteration, while decrementing by maxUnavailable on each
        # iteration, until all GameServers have been switched from one version to another.
        # "Recreate" terminates all non-allocated GameServers, and starts up a new set with the new details to replace them.
        type = "Recreate"
      }
      template = {
        metadata = {
          labels = {
            app = "mr_server"
          }
        }
        spec = {
          ports = [
            {
              name = "MUDDLE_LISTEN_PORT"
              # portPolicy has three options:
              # - "Dynamic" (default) the system allocates a free hostPort for the gameserver, for game clients to connect to
              # - "Static", user defines the hostPort that the game client will connect to. Then onus is on the user to ensure that the
              # port is available. When static is the policy specified, `hostPort` is required to be populated
              # - "Passthrough" dynamically sets the `containerPort` to the same value as the dynamically selected hostPort.
              #      This will mean that users will need to lookup what port has been opened through the server side SDK.
              portPolicy = "Passthrough"
              # protocol being used. Defaults to UDP. TCP and TCPUDP are other options
              # - "UDP" (default) use the UDP protocol
              # - "TCP", use the TCP protocol
              # - "TCPUDP", uses both TCP and UDP, and exposes the same hostPort for both protocols.
              #       This will mean that it adds an extra port, and the first port is set to TCP, and second port set to UDP
              protocol = "TCPUDP"
            }
          ]
          health = {
            # Number of seconds after the container has started before health check is initiated. Defaults to 5 seconds
            initialDelaySeconds : 5
            # If the `Health()` function doesn't get called at least once every period (seconds), then
            # the game server is not healthy. Defaults to 5
            periodSeconds : 5
            # Minimum consecutive failures for the health probe to be considered failed after having succeeded.
            # Defaults to 3. Minimum value is 1
            failureThreshold : 3
          }
          # Parameters for game server sidecar
          sdkServer = {
            logLevel = "Info"
            grpcPort = 9357
            httpPort = 9358
          }
          # Pod template configuration
          # https://v1-20.docs.kubernetes.io/docs/reference/generated/kubernetes-api/v1.20/#podtemplate-v1-core
          template = {
            metadata = {
              labels = {
                app = "mr_server"
              }
            }
            spec = {
              nodeSelector = {
                app = "mr_server"
              }
              tolerations = [
                {
                  key      = "app"
                  operator = "Equal"
                  value    = "mr_server"
                  effect   = "NoExecute"
                }
              ]
              containers = [
                {
                  name            = "mr-server"
                  image           = "mvlabat/mr_server"
                  imagePullPolicy = "Always"
                  resources = {
                    requests = {
                      memory = "64Mi"
                      cpu    = "500m"
                    }
                    limits = {
                      memory = "64Mi"
                      cpu    = "500m"
                    }
                  }
                  env = [
                    {
                      name = "SENTRY_DSN"
                      valueFrom = {
                        secretKeyRef = {
                          name = "sentry-dsn"
                          key  = "server"
                        }
                      }
                    }
                  ]
                }
              ]
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_manifest" "mr_server_fleet_autoscaler" {
  manifest = {
    # Reference: https://agones.dev/site/docs/reference/fleetautoscaler/
    apiVersion = "autoscaling.agones.dev/v1"
    kind       = "FleetAutoscaler"
    metadata = {
      name      = "mr-server"
      namespace = "default"
    }
    spec = {
      fleetName = "mr-server"
      policy = {
        type = "Webhook"
        webhook = {
          service = {
            name      = "mr-autoscaler-webhook-service"
            namespace = "default"
            port      = 8081
          }
        }
      }
      sync = {
        type = "FixedInterval"
        fixedInterval = {
          seconds = 5
        }
      }
    }
  }

  depends_on = [kubernetes_manifest.mr_server_fleet]
}
