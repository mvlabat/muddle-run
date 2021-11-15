# https://docs.aws.amazon.com/eks/latest/userguide/aws-load-balancer-controller.html

variable "cluster_name" {
  type = string
}

variable "region_account" {
  type = number
}

locals {
  cluster_oidc_issuer = replace((data.aws_eks_cluster.current.identity[*].oidc[0].issuer)[0], "https://", "")
}

data "aws_region" "current" {}

data "aws_caller_identity" "current" {}

data "aws_eks_cluster" "current" {
  name = var.cluster_name
}

resource "aws_iam_policy" "iam_policy" {
  name   = "AWSLoadBalancerControllerIAMPolicy"
  policy = file("${path.module}/iam_policy.json")
}

data "aws_iam_policy" "current" {
  name       = "AWSLoadBalancerControllerIAMPolicy"
  depends_on = [aws_iam_policy.iam_policy]
}

resource "aws_iam_role" "iam_role" {
  name = "AmazonEKSLoadBalancerControllerRole"
  assume_role_policy = jsonencode({
    "Version" : "2012-10-17",
    "Statement" : [
      {
        "Effect" : "Allow",
        "Principal" : {
          "Federated" : "arn:aws:iam::${data.aws_caller_identity.current.account_id}:oidc-provider/${local.cluster_oidc_issuer}"
        },
        "Action" : "sts:AssumeRoleWithWebIdentity",
        "Condition" : {
          "StringEquals" : {
            "${local.cluster_oidc_issuer}:sub" : "system:serviceaccount:kube-system:aws-load-balancer-controller"
          }
        }
      }
    ]
  })
}

resource "aws_iam_policy_attachment" "policy_attachment" {
  name       = "aws-load-balancer-controller-policy_attachment"
  roles      = [aws_iam_role.iam_role.name]
  policy_arn = data.aws_iam_policy.current.arn
}

resource "kubernetes_service_account" "service_account" {
  metadata {
    labels = {
      "app.kubernetes.io/component" = "controller"
      "app.kubernetes.io/name"      = "aws-load-balancer-controller"
    }
    name      = "aws-load-balancer-controller"
    namespace = "kube-system"
    annotations = {
      "eks.amazonaws.com/role-arn" = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:role/AmazonEKSLoadBalancerControllerRole"
    }
  }

  depends_on = [aws_iam_policy_attachment.policy_attachment]
}

resource "helm_release" "aws_load_balancer_controller" {
  name       = "aws-load-balancer-controller"
  chart      = "aws-load-balancer-controller"
  repository = "https://aws.github.io/eks-charts"
  namespace  = "kube-system"

  set {
    name  = "clusterName"
    value = var.cluster_name
  }
  set {
    name  = "serviceAccount.create"
    value = "false"
  }
  set {
    name  = "serviceAccount.name"
    value = "aws-load-balancer-controller"
  }
  set {
    name  = "image.repository"
    value = "${var.region_account}.dkr.ecr.${data.aws_region.current.name}.amazonaws.com/amazon/aws-load-balancer-controller"
  }

  depends_on = [kubernetes_service_account.service_account]
}
