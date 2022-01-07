variable "hosted_zone_name" {
  type = string
}

variable "record_name" {
  type    = string
  default = ""
}

data "aws_route53_zone" "current" {
  name = var.hosted_zone_name
}

data "aws_lb" "public" {
  tags = {
    "ingress.k8s.aws/stack" = "default/mr-service"
  }
}

resource "aws_route53_record" "www" {
  name            = var.record_name == "" ? var.hosted_zone_name : var.record_name
  type            = "A"
  zone_id         = data.aws_route53_zone.current.id
  allow_overwrite = true

  alias {
    evaluate_target_health = true
    name                   = data.aws_lb.public.dns_name
    zone_id                = data.aws_lb.public.zone_id
  }
}
