# Low Poly Earth Privacy Policy

Effective date: 1 October 2026

## Overview

Low Poly Earth is a desktop Earth explorer. It has no account system, advertising, tracking identifiers, or built-in analytics or crash-report upload service. Apart from the map data host described below, the developer does not operate a server that receives your activity from the app.

## Your location

The app does not request or access your device’s location. Places you explore by searching or navigating manually can trigger map and terrain requests for those areas when online data sources are enabled.

## Network data providers

The app requests terrain from Mapterhorn and map features from the developer's copy of Protomaps map data, hosted on Cloudflare at maps.lowpolyearth.online. Bundled imagery, the place-search index, star data, and simulated weather are processed locally.

Providers and their hosting services receive your public IP address, request time, requested tile coordinates or geographic bounds, or byte ranges identifying map data. Requests may also identify the app and its version. These services can infer which areas you explore and may keep server logs under their own policies. The developer cannot specify or control their retention periods or delete their records. The developer does not keep request logs for the map data host; Cloudflare, which serves it, processes requests under its own policies.

Use Settings > Data sources to control the providers used for map rendering. Requests already in progress may finish, and startup may contact the map data host to open its data archive even when its rendering source is disabled. Disconnecting your Mac from the network prevents remote requests; bundled data remains available with reduced detail.

## Local search and storage

Place-search text is searched against the bundled index on your Mac. It is not sent to an online geocoding service. Choosing a result can trigger data requests for that area.

Preferences, downloaded map and terrain caches, generated world data, and diagnostic logs are stored on your Mac. Local logs and caches can contain explored locations, request URLs, performance information, and local file paths. They are not automatically uploaded. Logs rotate when their size limit is reached; caches are managed according to the app's storage settings.

Settings > Storage provides controls to clear downloaded data on the next launch. Preferences and logs remain until you remove them. Sandboxed builds keep these files in the app's macOS container. Removing the app alone may leave local data behind.

## Photo exports

Photo export saves a rendered image and a JSON recipe to Pictures/Low Poly Earth. The recipe includes the scene's geographic position, camera settings, and simulated time; the image may also reveal the place shown. Nothing is uploaded by the export action. If you share these files, their recipients can see that information. Files in a folder synced by another service are subject to that service's settings.

The app requests Pictures access to write exports. It does not scan your existing photo collection. You can delete exported files using Finder.

## External links and support

For app support or privacy enquiries, email contact.urbanphilosophy@gmail.com. Opening a provider or other web link uses your browser, and the destination's privacy policy applies. If you choose to send diagnostic logs, screenshots, or photo recipes for support, review them first for locations and other information you do not wish to share.

When you email us, we receive your email address, any sender name, your message, and any attachments you choose to include. We use this information to respond to your enquiry and investigate issues you report, not for advertising. Correspondence is handled through Gmail and retained as needed for your enquiry and follow-up support. You can request deletion of support correspondence at the same address; records that must be retained for legal obligations may be kept. Your email provider and Google process messages under their own policies.

## Changes

Updates that change these practices will include an updated policy. The effective date identifies this version.
