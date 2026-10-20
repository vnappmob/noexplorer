// Presets for the account form: S3 providers (AWS, R2, B2…) and Nextcloud/WebDAV. What the form
// should suggest and show, and how an account is summarized in lists. The backend does the real
// work (provider.rs; s3.rs for AWS regions; webdav.rs for Nextcloud/WebDAV).

import type { Experimental, Profile, ProviderKind } from "./types";

export type ProviderId =
  | "custom"
  | "aws"
  | "r2"
  | "b2"
  | "wasabi"
  | "do"
  | "oci"
  | "nextcloud"
  | "webdav"
  | "box"
  | "onedrive"
  | "gcs"
  | "azure"
  | "sftp";

/** Providers shown in the picker only with their Settings → Experimental switch on. */
export const EXPERIMENTAL_PROVIDERS: Partial<Record<ProviderId, keyof Experimental>> = {
  box: "box_beta",
};

type Preset = {
  label: string;
  /** Backend provider (`Profile.kind`). */
  kind: ProviderKind;
  /** Shown under the provider picker. */
  hint: string;
  endpointPlaceholder: string;
  endpointRequired: boolean;
  regionPlaceholder: string;
  /** Region to fill in when switching to this provider (if the field is empty). */
  region: string;
  pathStyle: boolean;
  /** Does an endpoint belong to this provider? */
  matches: (host: string) => boolean;
};

export const PRESETS: Record<ProviderId, Preset> = {
  custom: {
    kind: "s3",
    label: "S3-compatible",
    hint: "MinIO, SeaweedFS, Garage, Ceph… Keep path-style on.",
    endpointPlaceholder: "https://minio.example.com or http://192.168.1.10:9000",
    endpointRequired: true,
    regionPlaceholder: "us-east-1",
    region: "us-east-1",
    pathStyle: true,
    matches: () => true,
  },
  aws: {
    kind: "s3",
    label: "AWS S3",
    hint: "Just the access key and secret (IAM → Users → Security credentials). Each bucket's region is found automatically.",
    endpointPlaceholder: "(leave empty: recommended)",
    endpointRequired: false,
    regionPlaceholder: "e.g. ap-southeast-1 (optional)",
    region: "",
    pathStyle: false,
    matches: (h) => h === "" || h.endsWith(".amazonaws.com") || h.endsWith(".amazonaws.com.cn"),
  },
  r2: {
    kind: "s3",
    label: "Cloudflare R2",
    hint: "Paste your Account ID (R2 overview page) and an API token's access key and secret (R2 → Manage API tokens). Or paste the S3 endpoint the token page shows: the form fills itself in.",
    endpointPlaceholder: "https://<ACCOUNT_ID>.r2.cloudflarestorage.com",
    endpointRequired: true,
    regionPlaceholder: "auto",
    region: "auto",
    pathStyle: true,
    matches: (h) => h.endsWith(".r2.cloudflarestorage.com"),
  },
  b2: {
    kind: "s3",
    label: "Backblaze B2",
    hint: "Pick your bucket's region (the bucket page shows its endpoint, e.g. s3.us-west-004.backblazeb2.com; pasting it works too) and an application key (keyID + applicationKey).",
    endpointPlaceholder: "https://s3.us-west-004.backblazeb2.com",
    endpointRequired: true,
    regionPlaceholder: "(from the endpoint)",
    region: "",
    pathStyle: true,
    matches: (h) => h.endsWith(".backblazeb2.com"),
  },
  wasabi: {
    kind: "s3",
    label: "Wasabi",
    hint: "Pick your bucket's region and an access key (Access Keys → Create new access key).",
    endpointPlaceholder: "https://s3.ap-southeast-1.wasabisys.com",
    endpointRequired: true,
    regionPlaceholder: "(from the endpoint)",
    region: "",
    pathStyle: true,
    matches: (h) => h.endsWith(".wasabisys.com"),
  },
  do: {
    kind: "s3",
    label: "DigitalOcean Spaces",
    hint: "Pick your Space's datacenter (sgp1, nyc3…) and a Spaces access key (API → Spaces Keys).",
    endpointPlaceholder: "https://sgp1.digitaloceanspaces.com",
    endpointRequired: true,
    regionPlaceholder: "(from the endpoint)",
    region: "",
    pathStyle: false,
    matches: (h) => h.endsWith(".digitaloceanspaces.com"),
  },
  nextcloud: {
    kind: "nextcloud",
    label: "Nextcloud",
    hint: "Your Nextcloud (or ownCloud) address. Sign in through the browser, or use your user name and an app password (Settings → Security → Devices & sessions).",
    endpointPlaceholder: "https://cloud.example.com",
    endpointRequired: true,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
  box: {
    kind: "box",
    label: "Box",
    hint: "Uses your own Box app: Box Developer Console → Create Platform App → Custom App → User Authentication (OAuth 2.0). Set the Redirect URI to http://127.0.0.1:53682/callback, tick Read and Write all files and folders, then paste the Client ID and Client secret here.",
    endpointPlaceholder: "",
    endpointRequired: false,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
  onedrive: {
    kind: "onedrive",
    label: "OneDrive",
    hint: "Sign in with your Microsoft account: personal, work or school. Nothing to set up; your password stays with Microsoft, and NoExplorer only gets access to your files. A work or school account may need your IT admin to allow NoExplorer first.",
    endpointPlaceholder: "",
    endpointRequired: false,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
  webdav: {
    kind: "webdav",
    label: "WebDAV",
    hint: "The full WebDAV folder URL from your server or NAS (Synology, QNAP, Apache, nginx…).",
    endpointPlaceholder: "https://nas.example.com:5006/webdav",
    endpointRequired: true,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
  oci: {
    kind: "s3",
    label: "Oracle Cloud",
    hint: "Uses Oracle's Amazon S3 Compatibility API. Enter your namespace (on any bucket's details page) and region; the access key and secret are a Customer Secret Key (Profile → User settings → Customer secret keys). Buckets made here go to the compartment set for the S3 API (the root one by default).",
    endpointPlaceholder: "https://<namespace>.compat.objectstorage.us-ashburn-1.oci.customer-oci.com",
    endpointRequired: true,
    regionPlaceholder: "(from the endpoint)",
    region: "",
    pathStyle: true,
    matches: (h) =>
      h.includes(".compat.objectstorage.") && (h.endsWith(".oci.customer-oci.com") || h.endsWith(".oraclecloud.com")),
  },
  gcs: {
    kind: "s3",
    label: "Google Cloud Storage",
    hint: "Uses the S3-compatible XML API with an HMAC key: Cloud Storage → Settings → Interoperability → Create a key. To list your buckets, enter the project ID (or set a default project for interoperable access on that page, or a default bucket).",
    endpointPlaceholder: "https://storage.googleapis.com",
    endpointRequired: true,
    regionPlaceholder: "auto",
    region: "auto",
    pathStyle: true,
    matches: (h) => h === "storage.googleapis.com",
  },
  azure: {
    kind: "azure",
    label: "Azure Blob Storage",
    hint: "Enter the storage account name and one of its keys (Storage account → Security + networking → Access keys). The endpoint fills itself in; change it only for another Azure cloud, a custom domain or Azurite.",
    endpointPlaceholder: "https://<account>.blob.core.windows.net",
    endpointRequired: false,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
  sftp: {
    kind: "sftp",
    label: "SFTP",
    hint: "Uses your SSH setup: keys, agent and ~/.ssh/config, like the ssh command. If the server asks for a password, or your key has a passphrase, enter it below (it's kept in your system keychain). Example: sftp://me@nas.local/volume1/backups",
    endpointPlaceholder: "sftp://user@host:22/folder",
    endpointRequired: true,
    regionPlaceholder: "",
    region: "",
    pathStyle: true,
    matches: () => false,
  },
};

export function endpointHost(endpoint: string) {
  const e = endpoint.trim().replace(/^[a-z]+:\/\//i, "");
  return e.split(/[/:]/)[0].toLowerCase();
}

/** Which preset an existing account looks like (S3 with an empty endpoint = AWS). */
export function guessProvider(p: Pick<Profile, "endpoint"> & { kind?: ProviderKind }): ProviderId {
  if (p.kind && p.kind !== "s3") return p.kind;
  const h = endpointHost(p.endpoint);
  for (const id of ["aws", "r2", "b2", "wasabi", "do", "gcs", "oci"] as const) if (PRESETS[id].matches(h)) return id;
  return "custom";
}

/** One-line summary for account lists: "AWS S3 · ap-southeast-1", "minio.example.com". */
export function describeAccount(p: Profile) {
  const id = guessProvider(p);
  if (id === "box") return "Box";
  if (id === "onedrive") return p.access_key ? `OneDrive · ${p.access_key}` : "OneDrive";
  if (id === "sftp") return `SFTP · ${endpointHost(p.endpoint)}`;
  if (id === "azure") return `Azure · ${endpointHost(azureEndpoint(p)).split(".")[0]}`;
  if (id === "nextcloud" || id === "webdav") return `${PRESETS[id].label} · ${endpointHost(p.endpoint)}`;
  if (id === "aws") return p.region ? `AWS S3 · ${p.region}` : "AWS S3";
  const host = endpointHost(p.endpoint);
  return id === "custom" ? host : `${PRESETS[id].label} · ${host.split(".")[0]}`;
}

/** Azure's Blob endpoint: the one entered, else the storage account's public one (azure.rs `blob_endpoint`). */
export const azureEndpoint = (p: Pick<Profile, "endpoint" | "access_key">) =>
  p.endpoint.trim().replace(/\/+$/, "") || `https://${p.access_key.trim().toLowerCase()}.blob.core.windows.net`;

/** Is this account S3 (buckets, regions, path-style)? */
export const isS3 = (p: Pick<Profile, "kind"> | null | undefined) => !p?.kind || p.kind === "s3";
/** Accounts with several containers (a bucket picker): S3 buckets, Azure containers. */
export const hasContainers = (p: Pick<Profile, "kind"> | null | undefined) => isS3(p) || p?.kind === "azure";

/** How a provider looks in the picker and the account list: a monogram badge on a hue, a short
 * line, and its group. Plain monograms, not logos: the names are the vendors' trademarks. */
export const LOOK: Record<ProviderId, { mono: string; hue: number; blurb: string; name: string }> = {
  aws: { mono: "S3", hue: 30, name: "Amazon S3", blurb: "Any region, detected per bucket" },
  custom: { mono: "S3", hue: 350, name: "MinIO & self-hosted", blurb: "MinIO, SeaweedFS, Garage, Ceph…" },
  r2: { mono: "R2", hue: 22, name: "Cloudflare R2", blurb: "S3 API token" },
  b2: { mono: "B2", hue: 2, name: "Backblaze B2", blurb: "Application key" },
  wasabi: { mono: "W", hue: 140, name: "Wasabi", blurb: "Access key" },
  do: { mono: "DO", hue: 210, name: "DigitalOcean Spaces", blurb: "Spaces key" },
  oci: { mono: "O", hue: 8, name: "Oracle Cloud", blurb: "Customer Secret Key" },
  gcs: { mono: "G", hue: 220, name: "Google Cloud Storage", blurb: "HMAC key + project" },
  azure: { mono: "Az", hue: 200, name: "Azure Blob Storage", blurb: "Account name + key" },
  nextcloud: { mono: "NC", hue: 205, name: "Nextcloud / ownCloud", blurb: "Sign in with your browser" },
  webdav: { mono: "DAV", hue: 250, name: "WebDAV", blurb: "NAS or any WebDAV server" },
  sftp: { mono: ">_", hue: 165, name: "SFTP", blurb: "Any SSH server" },
  box: { mono: "box", hue: 212, name: "Box", blurb: "Your own Box app · beta" },
  onedrive: { mono: "OD", hue: 214, name: "OneDrive", blurb: "Sign in with Microsoft" },
};

export const PICKER_GROUPS: { title: string; ids: ProviderId[] }[] = [
  { title: "Object storage", ids: ["aws", "custom", "r2", "b2", "wasabi", "do", "gcs", "azure", "oci"] },
  { title: "File servers", ids: ["nextcloud", "webdav", "sftp"] },
  { title: "Cloud drives", ids: ["onedrive", "box"] },
];

/** A provider badge (`.pbadge`); `sm` for lists. */
export const badge = (id: ProviderId, sm = false) =>
  `<span class="pbadge${sm ? " sm" : ""}${LOOK[id].mono.length > 2 ? " long" : ""}" style="--h:${LOOK[id].hue}" aria-hidden="true">${LOOK[id].mono}</span>`;

// ---------- simple setup: build the endpoint from what the provider's console shows ----------

export type SetupField = { name: string; label: string; placeholder: string; options?: string[] };
export type Setup = {
  /** Fields to type instead of an endpoint (none = nothing to type). */
  fields: SetupField[];
  /** The endpoint for these values ("" while a field is missing). */
  endpoint: (v: Record<string, string>) => string;
  /** Values back from a saved endpoint; null = a custom endpoint the simple form can't show. */
  parse: (endpoint: string) => Record<string, string> | null;
};

const host = (e: string) =>
  e
    .trim()
    .replace(/^[a-z]+:\/\//i, "")
    .replace(/\/+$/, "")
    .toLowerCase();
const blank = (e: string): Record<string, string> | null => (host(e) ? null : {});
const all = (v: Record<string, string>, names: string[]) => names.every((n) => v[n]?.trim());

const B2_REGIONS = ["us-west-000", "us-west-001", "us-west-002", "us-west-004", "us-east-005", "eu-central-003"];
const WASABI_REGIONS = [
  "us-east-1",
  "us-east-2",
  "us-central-1",
  "us-west-1",
  "ca-central-1",
  "eu-central-1",
  "eu-central-2",
  "eu-west-1",
  "eu-west-2",
  "eu-west-3",
  "eu-south-1",
  "ap-northeast-1",
  "ap-northeast-2",
  "ap-southeast-1",
  "ap-southeast-2",
];
const DO_REGIONS = ["nyc3", "sfo2", "sfo3", "ams3", "fra1", "lon1", "tor1", "sgp1", "syd1", "blr1", "atl1"];
const OCI_REGIONS = [
  "us-ashburn-1",
  "us-phoenix-1",
  "us-sanjose-1",
  "us-chicago-1",
  "ca-toronto-1",
  "ca-montreal-1",
  "mx-queretaro-1",
  "mx-monterrey-1",
  "sa-saopaulo-1",
  "sa-vinhedo-1",
  "sa-santiago-1",
  "sa-bogota-1",
  "uk-london-1",
  "uk-cardiff-1",
  "eu-frankfurt-1",
  "eu-amsterdam-1",
  "eu-paris-1",
  "eu-marseille-1",
  "eu-zurich-1",
  "eu-stockholm-1",
  "eu-milan-1",
  "eu-madrid-1",
  "me-dubai-1",
  "me-abudhabi-1",
  "me-jeddah-1",
  "me-riyadh-1",
  "il-jerusalem-1",
  "af-johannesburg-1",
  "ap-mumbai-1",
  "ap-hyderabad-1",
  "ap-singapore-1",
  "ap-singapore-2",
  "ap-tokyo-1",
  "ap-osaka-1",
  "ap-seoul-1",
  "ap-chuncheon-1",
  "ap-sydney-1",
  "ap-melbourne-1",
  "ap-batam-1",
];

export const SETUP: Partial<Record<ProviderId, Setup>> = {
  aws: { fields: [], endpoint: () => "", parse: (e) => (host(e) ? null : {}) },
  gcs: {
    fields: [],
    endpoint: () => "https://storage.googleapis.com",
    parse: (e) => (["", "storage.googleapis.com"].includes(host(e)) ? {} : null),
  },
  r2: {
    fields: [{ name: "account", label: "Account ID", placeholder: "On the R2 overview page, right side" }],
    endpoint: (v) => (all(v, ["account"]) ? `https://${v.account.trim().toLowerCase()}.r2.cloudflarestorage.com` : ""),
    parse: (e) => {
      const m = host(e).match(/^([a-z0-9]+)\.r2\.cloudflarestorage\.com$/);
      return m ? { account: m[1] } : blank(e);
    },
  },
  b2: {
    fields: [
      { name: "region", label: "Region", placeholder: "e.g. us-west-004 (on the bucket page)", options: B2_REGIONS },
    ],
    endpoint: (v) => (all(v, ["region"]) ? `https://s3.${v.region.trim()}.backblazeb2.com` : ""),
    parse: (e) => {
      const m = host(e).match(/^s3\.([a-z0-9-]+)\.backblazeb2\.com$/);
      return m ? { region: m[1] } : blank(e);
    },
  },
  wasabi: {
    fields: [{ name: "region", label: "Region", placeholder: "e.g. ap-southeast-1", options: WASABI_REGIONS }],
    endpoint: (v) => (all(v, ["region"]) ? `https://s3.${v.region.trim()}.wasabisys.com` : ""),
    parse: (e) => {
      const m = host(e).match(/^s3\.([a-z0-9-]+)\.wasabisys\.com$/);
      return m ? { region: m[1] } : blank(e);
    },
  },
  do: {
    fields: [{ name: "region", label: "Region", placeholder: "e.g. sgp1", options: DO_REGIONS }],
    endpoint: (v) => (all(v, ["region"]) ? `https://${v.region.trim()}.digitaloceanspaces.com` : ""),
    parse: (e) => {
      const m = host(e).match(/^([a-z0-9]+)\.digitaloceanspaces\.com$/);
      return m ? { region: m[1] } : blank(e);
    },
  },
  oci: {
    fields: [
      { name: "namespace", label: "Namespace", placeholder: "On any bucket's details page" },
      { name: "region", label: "Region", placeholder: "e.g. us-ashburn-1", options: OCI_REGIONS },
    ],
    endpoint: (v) =>
      all(v, ["namespace", "region"])
        ? `https://${v.namespace.trim().toLowerCase()}.compat.objectstorage.${v.region.trim()}.oci.customer-oci.com`
        : "",
    parse: (e) => {
      const m = host(e).match(
        /^([a-z0-9-]+)\.compat\.objectstorage\.([a-z0-9-]+)\.(oci\.customer-oci|oraclecloud)\.com$/,
      );
      return m ? { namespace: m[1], region: m[2] } : blank(e);
    },
  },
};
