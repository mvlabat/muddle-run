terraform {
  required_version = ">= 1.0.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 3.0"
    }
  }
}

variable "cluster_name" {
  type = string
}

data "aws_availability_zones" "available" {
}

data "aws_eks_cluster" "eks" {
  name = module.eks.cluster_id
}

data "aws_eks_cluster_auth" "eks" {
  name = module.eks.cluster_id
}

provider "kubernetes" {
  host                   = data.aws_eks_cluster.eks.endpoint
  cluster_ca_certificate = base64decode(data.aws_eks_cluster.eks.certificate_authority[0].data)
  token                  = data.aws_eks_cluster_auth.eks.token
}

resource "aws_security_group" "worker_group_mgmt_one" {
  name_prefix = "worker_group_mgmt_one"
  vpc_id      = module.vpc.vpc_id

  ingress {
    from_port = 22
    to_port   = 22
    protocol  = "tcp"

    cidr_blocks = [
      "10.0.0.0/8",
    ]
  }
  ingress {
    from_port = 7000
    to_port   = 8000
    protocol  = "udp"

    cidr_blocks = [
      "0.0.0.0/0",
    ]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 3.0"

  name                 = "muddle-run-vpc"
  cidr                 = "10.0.0.0/16"
  azs                  = data.aws_availability_zones.available.names
  public_subnets       = ["10.0.4.0/24", "10.0.5.0/24", "10.0.6.0/24"]
  enable_dns_hostnames = false

  tags = {
    "kubernetes.io/cluster/${var.cluster_name}" = "shared"
  }

  public_subnet_tags = {
    "kubernetes.io/cluster/${var.cluster_name}" = "shared"
    "kubernetes.io/role/elb"                    = "1"
  }
}

module "eks" {
  source           = "git::github.com/terraform-aws-modules/terraform-aws-eks.git?ref=v17.22.0"
  cluster_name     = var.cluster_name
  subnets          = module.vpc.public_subnets
  vpc_id           = module.vpc.vpc_id
  cluster_version  = "1.21"
  write_kubeconfig = false
  enable_irsa      = true

  worker_groups_launch_template = [
    {
      name                          = "default"
      override_instance_types       = ["t3a.medium", "t3.medium", "t2.medium"]
      asg_desired_capacity          = 3
      asg_min_size                  = 3
      asg_max_size                  = 4
      additional_security_group_ids = [aws_security_group.worker_group_mgmt_one.id]
      public_ip                     = true

      kubelet_extra_args = "--node-labels=node.kubernetes.io/lifecycle=`curl -s http://169.254.169.254/latest/meta-data/instance-life-cycle`"
    },
    {
      name                    = "game-server-workers"
      override_instance_types = ["c5a.large", "c5.large", "c4.large"]
      asg_desired_capacity    = 1
      asg_min_size            = 0
      asg_max_size            = 3
      public_ip               = true

      tags = [
        {
          key                 = "k8s.io/cluster-autoscaler/enabled"
          propagate_at_launch = "false"
          value               = "true"
        },
        {
          key                 = "k8s.io/cluster-autoscaler/muddle-run"
          propagate_at_launch = "false"
          value               = "owned"
        },
        {
          key                 = "k8s.io/cluster-autoscaler/node-template/label/app"
          propagate_at_launch = "true"
          value               = "mr_server"
        },
      ]

      kubelet_extra_args = "--node-labels=app=mr_server,node.kubernetes.io/lifecycle=`curl -s http://169.254.169.254/latest/meta-data/instance-life-cycle` --register-with-taints=app=mr_server:NoExecute"
    },
    // Node Pools with taints for metrics and system
    {
      name                 = "agones-system"
      instance_type        = "t3a.small"
      asg_desired_capacity = 1
      kubelet_extra_args   = "--node-labels=agones.dev/agones-system=true,node.kubernetes.io/lifecycle=`curl -s http://169.254.169.254/latest/meta-data/instance-life-cycle` --register-with-taints=agones.dev/agones-system=true:NoExecute"
      public_ip            = true
    },
    {
      name                 = "agones-metrics"
      instance_type        = "t3a.small"
      asg_desired_capacity = 1
      kubelet_extra_args   = "--node-labels=agones.dev/agones-metrics=true,node.kubernetes.io/lifecycle=`curl -s http://169.254.169.254/latest/meta-data/instance-life-cycle` --register-with-taints=agones.dev/agones-metrics=true:NoExecute"
      public_ip            = true
    }
  ]
}
