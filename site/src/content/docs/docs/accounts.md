---
title: Connect your storage
description: Add S3 (AWS, R2, B2, Wasabi, Google Cloud, Oracle…), Azure Blob, Nextcloud, WebDAV, SFTP, OneDrive and (experimental) Box accounts.
sidebar:
  order: 2
---

Add as many accounts as you like in **Settings → Accounts** (<kbd>⌘ ,</kbd>), then pick one from the account menu at the top of the right pane.

Passwords and secret keys go into your system's credential store (macOS Keychain, Windows Credential Manager or Linux Secret Service). They are only ever sent to the server you entered.

## S3 and compatible services

Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, DigitalOcean Spaces, MinIO, SeaweedFS, Garage, Ceph and anything else that speaks S3. Pick your provider first: the form then shows what it needs.

| Field | What to enter |
| --- | --- |
| Endpoint | Your server, e.g. `https://minio.example.com` or `http://192.168.1.10:9000`. **Leave it empty for Amazon S3**: each bucket's region is found automatically. |
| Access key, Secret key | The credentials from your provider. |
| Region | `us-east-1` works for most self-hosted servers. |
| Path-style requests | Leave it on for MinIO and other self-hosted servers. |

Click **Test**, then **Save & use**, and pick a bucket.

**No endpoint to look up for the big providers.** For Amazon S3, Cloudflare R2, Backblaze B2, Wasabi,
DigitalOcean Spaces, Google Cloud Storage and Oracle Cloud the form asks only for what their
console shows (an account ID, a namespace, a region picked from a list) and builds the endpoint
for you; it's shown under the fields. Pasting an endpoint works too: paste it in any field (or in
**Custom S3** → Endpoint) and NoExplorer picks the provider and fills the fields. **Custom
endpoint…** under the fields lets you type one yourself (a private link, a different domain).

## Nextcloud and ownCloud

1. Pick **Nextcloud / ownCloud** and enter your server address.
2. Click **Sign in with Nextcloud…** and approve NoExplorer in your browser.

NoExplorer gets its own app password. You never type your Nextcloud password into the app.

## WebDAV and NAS

Pick **WebDAV** and enter the full folder URL from your server or NAS (Synology, QNAP, Apache, nginx…), your user name and password.

The address must be the NAS's **WebDAV** address, not the one you open in a browser to manage it. The web page answers too, but it can't list or store files: NoExplorer then says *This address isn't a WebDAV folder*, and uploads fail with HTTP 405.

- **Synology:** install **WebDAV Server** from Package Center and turn on HTTPS (port 5006). Use `https://<nas>:5006/<shared folder>`, and give your user read/write on that shared folder (Control Panel → Shared Folder → Edit → Permissions).
- **QNAP:** Control Panel → Network & File Services → **WebDAV** → enable it, then set WebDAV permissions per shared folder. Use `https://<nas>/<shared folder>` (or the port shown there).
- **Behind your own nginx or reverse proxy:** the proxy has to pass WebDAV methods (`PROPFIND`, `PUT`, `MKCOL`, `MOVE`, `COPY`, `DELETE`) through to the NAS's WebDAV port. nginx's own pages answer them with *405 Not Allowed*. Point the proxy at port 5005/5006 (Synology), or connect to that port directly.

## OneDrive

Pick **OneDrive** and click **Sign in with Microsoft…**. Your browser opens Microsoft's own sign-in page; approve NoExplorer and you're done. There's no app to create and no key to copy.

- Personal Microsoft accounts work straight away.
- Work or school accounts: NoExplorer is a verified publisher app (VNAppMob), which most organizations let you approve yourself. If Microsoft still says you need approval, ask your IT admin to allow **NoExplorer**.
- NoExplorer asks only for your files (`Files.ReadWrite`), your basic profile (to name the account) and to stay signed in. Your password never reaches the app; the sign-in is kept in your system keychain.
- Deleted items go to the OneDrive recycle bin. **Copy link** makes a view-only link (with an expiry where your plan allows one).

## Box (experimental)

Turn on **Settings → Experimental → Box accounts** to see Box in the provider list. It hasn't been tried on a real Box account yet.

Box needs your own free Box app, because an open-source app can't keep a client secret private. It takes two minutes:

1. In the [Box Developer Console](https://app.box.com/developers/console), choose **Create Platform App → Custom App → User Authentication (OAuth 2.0)**.
2. Set the **Redirect URI** to `http://127.0.0.1:53682/callback` and tick *Read and write all files and folders*.
3. In NoExplorer, pick **Box**, paste the **Client ID** and **Client secret**, and click **Sign in with Box…**.

## Azure Blob Storage

Pick **Azure Blob Storage**, then enter the storage account name and one of its keys (**Storage account → Security + networking → Access keys**). That's all: the endpoint fills itself in as `https://<account>.blob.core.windows.net`. Change it only for another Azure cloud, a custom domain, or the Azurite emulator. Containers show up like buckets.

## Oracle Cloud

Oracle Cloud Object Storage works through its Amazon S3 Compatibility API. Pick **Oracle Cloud**, then:

- **Namespace and Region:** the namespace is on any bucket's details page; pick the region (like `us-ashburn-1`) from the list. NoExplorer builds the endpoint `https://<namespace>.compat.objectstorage.<region>.oci.customer-oci.com` (pasting it, or an older `…oraclecloud.com` one, also works).
- **Access key / Secret key:** a **Customer Secret Key** (Profile → User settings → Customer secret keys). Oracle shows the secret only once.

Buckets created through the S3 API go to the compartment chosen for it in Oracle's settings (the root compartment by default), and only buckets in that compartment are listed.

## Google Cloud Storage

Uses Google's S3-compatible API. In the Cloud console: **Cloud Storage → Settings → Interoperability → Create a key**. Endpoint `https://storage.googleapis.com`, region `auto`, and the key's access ID and secret.

To list your buckets, Google needs to know the project: fill in **Project ID** (shown on the console's project picker), or set a *default project for interoperable access* on the same Interoperability page, or enter a **Default bucket**. Otherwise the test says "Project number must be specified".

## SFTP

Enter an address like `sftp://me@nas.local/volume1/backups`. NoExplorer connects with the `ssh` command, so your SSH keys, agent and `~/.ssh/config` apply, just like in a terminal.

- **SSH keys (recommended):** leave the password empty. A key in `~/.ssh` or your agent is used on its own; for another one, fill in **Private key file** (or click **Choose…**).
- **Key with a passphrase:** put the passphrase in **Password or key passphrase**.
- **Password login:** put the server password in the same field.

The password or passphrase is kept in your system keychain, and only handed to `ssh` when it asks. A server you've never connected to is remembered on first use; a changed server key is refused.

## Self-signed HTTPS (experimental)

With **Settings → Experimental → Self-signed HTTPS** on, an `https://` account shows **Trust certificate…**. NoExplorer reads the server's certificate fingerprint; compare it with the one your server shows and confirm. From then on that account accepts exactly that certificate, and everything else is still checked normally.
