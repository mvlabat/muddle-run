variable "persistence_db_password" {
  type      = string
  sensitive = true
}

variable "vpc_id" {
  type = string
}

variable "vpc_public_subnets" {
  type = set(string)
}

variable "worker_group_mgmt_one_sg_id" {
  type = string
}


resource "aws_db_subnet_group" "persistence" {
  name       = "persistence"
  subnet_ids = var.vpc_public_subnets

  tags = {
    Name = "mr-persistence"
  }
}

resource "aws_security_group" "rds" {
  name_prefix = "persistence_db"
  vpc_id      = var.vpc_id

  ingress {
    from_port = 5432
    to_port   = 5432
    protocol  = "tcp"

    security_groups = [var.worker_group_mgmt_one_sg_id]
  }
}

resource "aws_db_parameter_group" "persistence" {
  name   = "persistence"
  family = "postgres13"

  parameter {
    name  = "log_connections"
    value = "1"
  }
}

resource "aws_db_instance" "persistence" {
  allocated_storage      = 5
  engine                 = "postgres"
  engine_version         = "13.4"
  instance_class         = "db.t4g.micro"
  name                   = "mr_persistence_production"
  username               = "postgres"
  password               = var.persistence_db_password
  db_subnet_group_name   = aws_db_subnet_group.persistence.name
  vpc_security_group_ids = [aws_security_group.rds.id]
  parameter_group_name   = aws_db_parameter_group.persistence.name
  skip_final_snapshot    = true
  multi_az               = true
  apply_immediately      = true
}
