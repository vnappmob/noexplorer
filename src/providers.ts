// Presets for the account form: S3 providers (AWS, R2, B2…) and Nextcloud/WebDAV. What the form
// should suggest and show, and how an account is summarized in lists. The backend does the real
// work (provider.rs; s3.rs for AWS regions; webdav.rs for Nextcloud/WebDAV).

import type { Profile, ProviderKind } from "./types";

export type ProviderId = "custom" | "aws" | "r2" | "b2" | "wasabi" | "do" | "nextcloud" | "webdav";

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
    hint: "Leave the endpoint empty. Each bucket's region is detected automatically; Region is only the default.",
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
    hint: "Endpoint: https://<ACCOUNT_ID>.r2.cloudflarestorage.com (R2 → Manage API tokens). Region: auto.",
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
    hint: "Endpoint from the bucket page, e.g. https://s3.us-west-004.backblazeb2.com. Region is read from it.",
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
    hint: "Endpoint for your bucket's region, e.g. https://s3.ap-southeast-1.wasabisys.com.",
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
    hint: "Endpoint: https://<region>.digitaloceanspaces.com, e.g. https://sgp1.digitaloceanspaces.com.",
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
};

export function endpointHost(endpoint: string) {
  const e = endpoint.trim().replace(/^[a-z]+:\/\//i, "");
  return e.split(/[/:]/)[0].toLowerCase();
}

/** Which preset an existing account looks like (S3 with an empty endpoint = AWS). */
export function guessProvider(p: Pick<Profile, "endpoint"> & { kind?: ProviderKind }): ProviderId {
  if (p.kind === "nextcloud" || p.kind === "webdav") return p.kind;
  const h = endpointHost(p.endpoint);
  for (const id of ["aws", "r2", "b2", "wasabi", "do"] as const) if (PRESETS[id].matches(h)) return id;
  return "custom";
}

/** One-line summary for account lists: "AWS S3 · ap-southeast-1", "minio.example.com". */
export function describeAccount(p: Profile) {
  const id = guessProvider(p);
  if (id === "nextcloud" || id === "webdav") return `${PRESETS[id].label} · ${endpointHost(p.endpoint)}`;
  if (id === "aws") return p.region ? `AWS S3 · ${p.region}` : "AWS S3";
  const host = endpointHost(p.endpoint);
  return id === "custom" ? host : `${PRESETS[id].label} · ${host.split(".")[0]}`;
}

/** Is this account S3 (buckets, regions, path-style)? */
export const isS3 = (p: Pick<Profile, "kind"> | null | undefined) => !p?.kind || p.kind === "s3";
