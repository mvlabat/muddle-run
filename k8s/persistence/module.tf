resource "kubernetes_deployment" "mr_persistence" {
  metadata {
    name = "mr-persistence"
  }

  spec {
    selector {
      match_labels = {
        service = "mr-persistence"
      }
    }
    replicas = 1
    template {
      metadata {
        labels = {
          app     = "muddle-run"
          service = "mr-persistence"
        }
      }
      spec {
        termination_grace_period_seconds = 5
        container {
          name              = "mr-persistence"
          image             = "mvlabat/mr_persistence:latest"
          image_pull_policy = "Always"
          port {
            name           = "persistence-pub"
            container_port = 8082
          }
          port {
            name           = "persistence"
            container_port = 8083
          }
          env {
            name = "SENTRY_DSN"
            value_from {
              secret_key_ref {
                name = "sentry-dsn"
                key  = "persistence"
              }
            }
          }
          env {
            name  = "DATABASE_URL"
            value = "postgres://postgres:${var.persistence_db_password}@${aws_db_instance.persistence.endpoint}"
          }
        }
      }
    }
  }
}
