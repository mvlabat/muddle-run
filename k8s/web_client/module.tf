resource "kubernetes_deployment" "mr_web_client" {
  metadata {
    name = "mr-web-client"
  }

  spec {
    selector {
      match_labels = {
        service = "mr-web-client"
      }
    }
    replicas = 2
    template {
      metadata {
        labels = {
          app     = "muddle-run"
          service = "mr-web-client"
        }
      }
      spec {
        termination_grace_period_seconds = 5
        container {
          name              = "mr-web-client"
          image             = "mvlabat/mr_web_client:latest"
          image_pull_policy = "Always"
          port {
            container_port = 80
          }
        }
      }
    }
  }
}
