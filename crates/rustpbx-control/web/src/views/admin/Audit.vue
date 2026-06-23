<script lang="ts">
// The audit view is identical for super-admin (all tenants) and tenant-admin
// (own tenant) — the component scopes its query by the active route name, so
// this is a thin re-export.
export { default } from "@/views/tadmin/Audit.vue";
</script>
