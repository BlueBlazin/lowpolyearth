# App Store privacy disclosure worksheet

Scope: the Apple silicon macOS application. Review alongside [the user-facing
policy](PRIVACY.md). App Store Connect answers are separate from the bundle's
privacy manifest.

## Answers established from the implementation

| Question or data type | Answer / behavior |
| --- | --- |
| Tracking across other companies' apps or websites | No tracking code, advertising identifiers, advertising SDKs, or data-broker integration |
| Advertising | None |
| Accounts, purchases, identifiers | No account or payment system; no app-generated persistent user/device identifier |
| Support correspondence | User-initiated email outside the app; Gmail receives the sender address/name, message, and chosen attachments |
| Search history | Search queries stay in the bundled local index; selecting a result requests data for its area |
| Photos and gameplay content | Local rendered PNG and JSON recipe exports only; no automatic upload |
| Diagnostics | Local logs and performance counters only; no automatic crash/analytics upload |
| Device location | The app does not request or access device location |
| Network request data | IP address, request time, app/version headers, tile coordinates, geographic bounds, and/or map byte ranges reach remote hosts |

The developer has no app collection backend; it hosts static map data on Cloudflare, which receives map requests. Support email is separate from
automatic app traffic. A blanket "Data Not Collected" answer is not established:
Apple counts off-device data retained beyond servicing a request, and provider
policies describe some logging. The following assessment distinguishes service
evidence from assumptions about particular API endpoints.

| Service | Endpoint host | When used |
| --- | --- | --- |
| Mapterhorn terrain | `tiles.mapterhorn.com`, `download.mapterhorn.com` | Default online terrain and its regional coverage index |
| Map data (developer-hosted Protomaps build) | `maps.lowpolyearth.online` (Cloudflare R2) | Startup archive open and default vector data |

## Provider policy findings

| Provider | Published evidence | Remaining uncertainty |
| --- | --- | --- |
| Mapterhorn terrain | Its [data access page](https://mapterhorn.com/data-access/) states that Cloudflare supports the project with R2 object storage, Workers, and bandwidth. | No Mapterhorn privacy or logging policy was found. Whether the project's hosting retains request logs, for how long, and for what purposes is not established. Cloudflare's own policies do not describe a customer's account configuration. |
| Map data | The developer hosts a copy of one Protomaps build in an R2 bucket behind Cloudflare's CDN, with no request logging configured. | Cloudflare's own processing of the requests follows its policies, not an account setting the developer controls. |

## Submission decisions

- No app advertising, tracking SDKs, accounts, or automatic diagnostic uploads
  were found. No provider evidence reviewed establishes advertising tracking
  of this app's requests; unresolved provider practices are not proof of it.
- Map requests are for App Functionality. If retained requests locate a device,
  assess Precise/Coarse Location; a manually explored place is not necessarily
  the user's location. IP-derived data categories depend on actual use.
- "Not linked to the user" is not established merely by having no accounts.
  Confirm whether request records retain identifying information and whether
  providers use them for analytics or other purposes before selecting these
  fields. Do not select every possible category as a substitute for evidence.
- Search text, preferences, photos, recipes, and local diagnostics stay on the
  device unless the user separately sends them. Website visits and external
  email are not automatic app collection. Reassess Apple's optional-disclosure
  criteria if an in-app support upload/form is introduced.

To finalize the outstanding provider fields, obtain endpoint-specific answers
on logged fields, retention, anonymization/linkage, and secondary uses, or use
providers with documented practices. No provider inquiries have been sent.
The public policy describes the uncertainty; the App Store collection label
remains unsubmitted and is not yet verified in full.

## Privacy manifest audit

`packaging/macos/PrivacyInfo.xcprivacy` is copied into `Contents/Resources` and
verified byte-for-byte. It declares no tracking and records these observed uses:

| Category | Reasons | Evidence |
| --- | --- | --- |
| File timestamp / metadata | `C617.1`, `3B52.1` | `stat`/`fstat`/`lstat` imports; app-container storage, log rotation, cache size, and user-authorized export directory metadata |
| System boot time | `35F9.1` | `mach_absolute_time`; Rust monotonic timers, frame timing, animation, request deadlines and local performance counters |

Apple's current required-reason API overview explicitly names iOS, iPadOS,
tvOS, visionOS and watchOS, not macOS. These truthful declarations are included
conservatively for the compiled macOS app; they are not a claim that the mobile
platform requirement applies to macOS. No disk-space or UserDefaults reason is
invented for APIs not observed in this audit.

The Cargo dependency inventory includes Rust `protobuf` (the independent
rust-protobuf implementation) and `ring` (which contains BoringSSL-derived
cryptographic code). Apple's listed-SDK rules include Protobuf and BoringSSL,
including repackaged SDKs. The app-bundle manifest records the combined
statically linked executable's API uses. Apple package validation still needs
to confirm whether it requires separate SDK manifest packaging for this build.
The manifest does not assert a completed data-collection label: collection
entries remain pending the provider assessment above.

## Public policy and App Store Connect

The policy is available offline through the app menu and Settings. Generate a
standalone page from the exact same Markdown:

```sh
python3 tools/write-privacy-policy.py target/privacy/PrivacyPolicy.html
```

Use https://lowpolyearth.online/privacy as the Privacy Policy URL in
App Store Connect. The public website is https://lowpolyearth.online/.
The hosted policy must match `docs/PRIVACY.md`; regenerate and upload the site
whenever the policy changes.

Use https://lowpolyearth.online/contact as the Support URL.
Support/privacy email: contact.urbanphilosophy@gmail.com.
The policy covers the voluntarily supplied correspondence and Gmail processing.
Enter these URLs and the verified disclosure answers in App Store Connect.
This worksheet has not been submitted to Apple.

## References

- [Apple's app privacy definitions](https://developer.apple.com/app-store/app-privacy-details/)
- [Required-reason API scope](https://developer.apple.com/documentation/bundleresources/describing-use-of-required-reason-api)
- [Approved API reasons](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitypereasons)
- [Third-party SDK requirements](https://developer.apple.com/support/third-party-SDK-requirements/)
