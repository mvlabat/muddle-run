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
    replicas = 2
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
            container_port = 8080
          }
        }
      }
    }
  }
}
