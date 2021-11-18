# https://docs.aws.amazon.com/eks/latest/userguide/network-load-balancing.html
# If this service gets stuck creating (processing finalizers), use the following command to enable force-deleting it:
# `kubectl patch service mr-matchmaker-service -p '{"metadata":{"finalizers":[]}}' --type=merge`
resource "kubernetes_service" "muddle-run-service" {
  metadata {
    name = "mr-service"
    annotations = {
      "service.beta.kubernetes.io/aws-load-balancer-type" : "external"
      "service.beta.kubernetes.io/aws-load-balancer-scheme" : "internet-facing"
      "service.beta.kubernetes.io/aws-load-balancer-nlb-target-type" : "ip"
    }
  }

  spec {
    type = "LoadBalancer"
    selector = {
      app = "muddle-run"
    }
    port {
      name = "http"
      port = 80
    }
    port {
      name = "ws"
      port = 8080
    }
  }
}
