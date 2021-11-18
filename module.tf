// Copyright 2020 Google LLC All Rights Reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.


// Run:
//  terraform apply [-var agones_version="1.4.0"]

// Install latest version of agones

terraform {
  required_version = ">= 1.0.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 3.63"
    }
    helm = {
      version = "~> 2.3"
      source  = "hashicorp/helm"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.6.1"
    }
  }
}

variable "agones_version" {
  default = "1.18.0"
}

variable "cluster_name" {
  default = "muddle-run"
}

variable "region" {
  default = "eu-central-1"
}

variable "region_account" {
  // https://docs.aws.amazon.com/eks/latest/userguide/add-ons-images.html
  default = 602401143452
}

provider "aws" {
  profile = "default"
  region  = var.region
}

data "aws_eks_cluster" "current" {
  name       = var.cluster_name
  depends_on = [module.eks_cluster]
}

data "aws_eks_cluster_auth" "current" {
  name       = var.cluster_name
  depends_on = [module.eks_cluster]
}

provider "helm" {
  kubernetes {
    host                   = data.aws_eks_cluster.current.endpoint
    cluster_ca_certificate = base64decode(data.aws_eks_cluster.current.certificate_authority[0].data)
    exec {
      api_version = "client.authentication.k8s.io/v1alpha1"
      args        = ["eks", "get-token", "--cluster-name", var.cluster_name]
      command     = "aws"
    }
  }
}

provider "kubernetes" {
  host                   = data.aws_eks_cluster.current.endpoint
  cluster_ca_certificate = base64decode(data.aws_eks_cluster.current.certificate_authority[0].data)
  exec {
    api_version = "client.authentication.k8s.io/v1alpha1"
    args        = ["eks", "get-token", "--cluster-name", var.cluster_name]
    command     = "aws"
  }
}

variable "log_level" {
  default = "info"
}

module "eks_cluster" {
  source       = "./k8s/eks_cluster"
  cluster_name = var.cluster_name
}

module "aws_load_balancer_controller" {
  source         = "./k8s/aws_load_balancer_controller"
  cluster_name   = var.cluster_name
  region_account = var.region_account

  depends_on = [data.aws_eks_cluster_auth.current]
}

module "aws_autoscaler" {
  source       = "./k8s/aws_autoscaler"
  cluster_name = var.cluster_name
  depends_on   = [module.eks_cluster]
}

module "matchmaker" {
  source     = "./k8s/matchmaker"
  depends_on = [module.aws_load_balancer_controller, module.helm_agones]
}

module "web_client" {
  source     = "./k8s/web_client"
  depends_on = [module.aws_load_balancer_controller]
}

module "service" {
  source     = "./k8s/service"
  depends_on = [module.matchmaker, module.web_client]
}

# Comment this out if running for the first time (i.e. when `helm_agones` is not installed).
module "agones" {
  source     = "./k8s/agones"
  depends_on = [module.eks_cluster, module.helm_agones]
}

// Next Helm module cause "terraform destroy" timeout, unless helm release would be deleted first.
// Therefore "helm delete --purge agones" should be executed from the CLI before executing "terraform destroy".
module "helm_agones" {
  source = "git::https://github.com/googleforgames/agones.git//install/terraform/modules/helm3/?ref=main"

  udp_expose             = "false"
  agones_version         = var.agones_version
  values_file            = ""
  feature_gates          = "PlayerTracking=true&StateAllocationFilter=true&PlayerAllocationFilter=true"
  host                   = data.aws_eks_cluster.current.endpoint
  token                  = data.aws_eks_cluster_auth.current.token
  cluster_ca_certificate = base64decode(data.aws_eks_cluster.current.certificate_authority[0].data)
  log_level              = var.log_level
}
