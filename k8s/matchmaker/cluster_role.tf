resource "kubernetes_cluster_role" "matchmaker_role" {
  metadata {
    name = "matchmaker-role"
  }

  rule {
    api_groups = [""]
    resources  = ["pods"]
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
