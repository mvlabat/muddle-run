resource "kubernetes_deployment" "mr_matchmaker" {
  metadata {
    name = "mr-matchmaker"
  }

  spec {
    selector {
      match_labels = {
        service = "mr-matchmaker"
      }
    }
    # We can't have more replicas until we implement state sharing.
    # To be able to serve the webhook autoscaler's endpoint correctly, we need to know how many clients there are
    # connected to all the matchmaker replicas in total.
    replicas = 1
    template {
      metadata {
        labels = {
          app     = "muddle-run"
          service = "mr-matchmaker"
        }
      }
      spec {
        termination_grace_period_seconds = 5
        container {
          name              = "mr-matchmaker"
          image             = "mvlabat/mr_matchmaker:latest"
          image_pull_policy = "Always"
          port {
            name           = "ws"
            container_port = 8080
          }
          port {
            name           = "webhook"
            container_port = 8081
          }
          env {
            name = "SENTRY_DSN"
            value_from {
              secret_key_ref {
                name = "sentry-dsn"
                key  = "matchmaker"
              }
            }
          }
        }
      }
    }
  }
}
