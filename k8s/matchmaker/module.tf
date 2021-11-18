resource "kubernetes_deployment" "mr_matchmaker" {
  metadata {
    name = "mr-matchmaker"
  }

  spec {
    selector {
      match_labels = {
        app = "mr-matchmaker"
      }
    }
    replicas = 2
    template {
      metadata {
        labels = {
          app = "mr-matchmaker"
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

# https://docs.aws.amazon.com/eks/latest/userguide/network-load-balancing.html
# If this service gets stuck creating (processing finalizers), use the following command to enable force-deleting it:
# `kubectl patch service mr-matchmaker-service -p '{"metadata":{"finalizers":[]}}' --type=merge`
resource "kubernetes_service" "mr_matchmaker_service" {
  metadata {
    name = "mr-matchmaker-service"
    annotations = {
      "service.beta.kubernetes.io/aws-load-balancer-type" : "external"
      "service.beta.kubernetes.io/aws-load-balancer-scheme" : "internet-facing"
      "service.beta.kubernetes.io/aws-load-balancer-nlb-target-type" : "ip"
    }
  }

  spec {
    type = "LoadBalancer"
    selector = {
      app = "mr-matchmaker"
    }
    port {
      port = 8080
    }
  }

  depends_on = [kubernetes_cluster_role_binding.matchmaker_role_binding]
}
