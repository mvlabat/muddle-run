variable "hosted_zone_name" {
  type = string
}

data "aws_acm_certificate" "current" {
  domain      = var.hosted_zone_name
  statuses    = ["ISSUED"]
  most_recent = true
}

resource "kubernetes_ingress" "muddle_run_service" {
  metadata {
    name = "mr-service"
    annotations = {
      "kubernetes.io/ingress.class" : "alb"
      "alb.ingress.kubernetes.io/scheme" : "internet-facing"
      "alb.ingress.kubernetes.io/certificate-arn" : "${data.aws_acm_certificate.current.arn}"
      "alb.ingress.kubernetes.io/listen-ports" : "[{\"HTTPS\":443}, {\"HTTP\":80}]"
      "alb.ingress.kubernetes.io/actions.ssl-redirect" : "{\"Type\": \"redirect\", \"RedirectConfig\": { \"Protocol\": \"HTTPS\", \"Port\": \"443\", \"StatusCode\": \"HTTP_301\"}}"
    }
  }

  spec {
    rule {
      host = "muddle.run"
      http {
        path {
          path = "/*"
          backend {
            service_name = "ssl-redirect"
            service_port = "use-annotation"
          }
        }

        path {
          path = "/matchmaker/*"
          backend {
            service_name = "mr-matchmaker"
            service_port = 8080
          }
        }

        path {
          path = "/persistence/*"
          backend {
            service_name = "mr-persistence"
            service_port = 8082
          }
        }

        path {
          path = "/*"
          backend {
            service_name = "mr-web-client"
            service_port = 80
          }
        }
      }
    }
  }
}

resource "kubernetes_service" "mr_web_client" {
  metadata {
    name = "mr-web-client"
  }
  spec {
    type = "NodePort"
    selector = {
      app = "muddle-run"
    }
    port {
      port = 80
    }
  }
}

resource "kubernetes_service" "mr_matchmaker" {
  metadata {
    name = "mr-matchmaker"
  }
  spec {
    type = "NodePort"
    selector = {
      app = "muddle-run"
    }
    port {
      port = 8080
    }
  }
}

resource "kubernetes_service" "mr_persistence" {
  metadata {
    name = "mr-persistence"
  }
  spec {
    type = "NodePort"
    selector = {
      app = "muddle-run"
    }
    port {
      port = 8082
    }
  }
}

# https://docs.aws.amazon.com/eks/latest/userguide/network-load-balancing.html
# If this service gets stuck creating (processing finalizers), use the following command to enable force-deleting it:
# `kubectl patch service mr-autoscaler-webhook-service -p '{"metadata":{"finalizers":[]}}' --type=merge`
resource "kubernetes_service" "muddle_run_autoscaler_webhook" {
  metadata {
    name = "mr-autoscaler-webhook-service"
    annotations = {
      "service.beta.kubernetes.io/aws-load-balancer-type" : "external"
      "service.beta.kubernetes.io/aws-load-balancer-scheme" : "internal"
      "service.beta.kubernetes.io/aws-load-balancer-nlb-target-type" : "ip"
      "service.beta.kubernetes.io/aws-load-balancer-cross-zone-load-balancing-enabled" : "true"
    }
  }

  spec {
    type = "LoadBalancer"
    selector = {
      app = "muddle-run"
    }
    port {
      name = "webhook"
      port = 8081
    }
    port {
      name = "persistence"
      port = 8083
    }
  }
}
