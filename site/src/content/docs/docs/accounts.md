---
title: Connect your storage
description: Add S3, Nextcloud, WebDAV, Box, and (experimental) Google Cloud Storage, Azure Blob and SFTP accounts.
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

## Nextcloud and ownCloud

1. Pick **Nextcloud / ownCloud** and enter your server address.
2. Click **Sign in with Nextcloud…** and approve NoExplorer in your browser.

NoExplorer gets its own app password. You never type your Nextcloud password into the app.

## WebDAV and NAS

Pick **WebDAV** and enter the full folder URL from your server or NAS (Synology, QNAP, Apache, nginx…), your user name and password.

## Box (beta)

Box needs your own free Box app, because an open-source app can't keep a client secret private. It takes two minutes:

1. In the [Box Developer Console](https://app.box.com/developers/console), choose **Create Platform App → Custom App → User Authentication (OAuth 2.0)**.
2. Set the **Redirect URI** to `http://127.0.0.1:53682/callback` and tick *Read and write all files and folders*.
3. In NoExplorer, pick **Box**, paste the **Client ID** and **Client secret**, and click **Sign in with Box…**.

## More storage types (experimental)

Turn on **Settings → Experimental → More storage types** to add these to the provider list.

### Google Cloud Storage

Uses Google's S3-compatible API. In the Cloud console: **Cloud Storage → Settings → Interoperability → Create a key**. Endpoint `https://storage.googleapis.com`, region `auto`, and the key's access ID and secret.

### Azure Blob Storage

Endpoint `https://<account>.blob.core.windows.net`, the storage account name, and one of its keys (**Storage account → Security + networking → Access keys**). Containers show up like buckets.

### SFTP

Enter an address like `sftp://me@nas.local/volume1/backups`. NoExplorer connects with the `ssh` command, so your SSH keys, agent and `~/.ssh/config` apply. Passwords aren't supported yet: set up key login first. A server you've never connected to is remembered on first use; a changed server key is refused.

### Self-signed HTTPS

With **Settings → Experimental → Self-signed HTTPS** on, an `https://` account shows **Trust certificate…**. NoExplorer reads the server's certificate fingerprint; compare it with the one your server shows and confirm. From then on that account accepts exactly that certificate, and everything else is still checked normally.
