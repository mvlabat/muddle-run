resource "kubernetes_cluster_role" "matchmaker_role" {
  metadata {
    name = "matchmaker-role"
  }

  rule {
    api_groups = ["", "agones.dev", "allocation.agones.dev"]
    resources  = ["pods", "gameservers"]
    verbs      = ["get", "watch", "list"]
  }
}

resource "kubernetes_cluster_role_binding" "matchmaker_role_binding" {
  metadata {
    name = "matchmaker-role-binding"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "matchmaker-role"
  }
  subject {
    kind = "ServiceAccount"
    name = "default"
  }

  depends_on = [kubernetes_cluster_role.matchmaker_role]
}

resource "kubernetes_cluster_role" "matchmaker_allocation_role" {
  metadata {
    name = "matchmaker-allocation-role"
  }

  rule {
    api_groups = ["allocation.agones.dev"]
    resources  = ["gameserverallocations"]
    verbs      = ["create"]
  }
}

resource "kubernetes_cluster_role_binding" "matchmaker_allocation_role_binding" {
  metadata {
    name = "matchmaker-allocation-role-binding"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "matchmaker-allocation-role"
  }
  subject {
    kind = "ServiceAccount"
    name = "default"
  }

  depends_on = [kubernetes_cluster_role.matchmaker_allocation_role]
}
