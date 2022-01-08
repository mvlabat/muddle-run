resource "kubernetes_cluster_role" "server_role" {
  metadata {
    name = "server-role"
  }

  rule {
    api_groups = [""]
    resources  = ["pods"]
    verbs      = ["get", "list"]
  }
}

resource "kubernetes_cluster_role_binding" "server_role_binding" {
  metadata {
    name = "server-role-binding"
  }
  role_ref {
    api_group = "rbac.authorization.k8s.io"
    kind      = "ClusterRole"
    name      = "server-role"
  }
  subject {
    kind = "ServiceAccount"
    name = "agones-sdk"
  }

  depends_on = [kubernetes_cluster_role.server_role]
}
