# Manga and mining parity matrix

`PASS` is evidence for this fork. Stock behavior remains `UNVERIFIED` where safely inspecting it would expose or mutate private stock state.

| Capability | Fork result | Remaining acceptance work |
| --- | --- | --- |
| Open/resume local manga | PASS | Long-session soak and unusual image-format sampling. |
| Continuous vertical reader | PASS (device) | User comparison of fling feel/memory against stock on long volumes. |
| RTL/LTR paging and swipe | PASS | More edge cases at volume boundaries. |
| Fit, pinch/toolbar zoom and pan | PASS (implementation; pan device) | Hands-on two-finger smoothness and transformed OCR-overlay alignment. Double-tap is intentionally disabled. |
| Scrollbar, bookmark, persistent Library button, next/previous volume | PASS | User UX review. Page previews are intentionally absent. |
| Two-page wide mode and rotation | PASS (build/rotation smoke) | Fold/unfold and offset-spread UX comparison. |
| Offline OCR preparation/cache | PASS, LOWER ACCURACY | Bundled ML Kit remains private but does not match Lens recognition quality. Detected regions now use Manatan's geometric merger; old caches are bypassed and explicit reprocessing forces new inference. |
| Explicit high-quality reader OCR | PASS (implementation) | Consented Google Lens path and provider-specific cache compile in the Android release; representative user-page acceptance remains. |
| Bulk series/volume OCR and offline copy | PASS | Large real-library soak. |
| OCR overlay and selection geometry | PARTIAL | Compact popup placement protects only the selected word; zoom clears active selection while OCR remains usable at the transformed scale. Experimental invisible-hit-region mode suppresses whole-block/transparent selection artifacts and paints a stable word-only highlight. End-to-end real-page placement and approximate boxless character alignment remain QA. |
| Dictionary import/order/enable/remove | PARTIAL | Large/cancel-safe tests and inherited deinflector failures. |
| Structured lookup/frequency/pitch/furigana/audio | PRESENT, UNVERIFIED | Full mined-word journey. |
| Context correction and image crop | PRESENT, UNVERIFIED | Full mined-card journey. |
| AnkiDroid mapping/tags/duplicates | PASS (synthetic) | Real reader selection flow, custom template/audio/crop/failure cases. |
| Categories/multiple membership | PASS (implementation) | Complete UI matrix. |
| Series/volume hidden visibility | PASS (implementation) | Exhaustive no-leak test across search/history/stats/backup. |
| Removable reading sessions by manga | PASS (installed API) | DST boundary and deletion UI review. Lookup counters and charts are intentionally absent. |
| Credential-free backup/restore | PASS (implementation) | Restore and source re-authorisation UX test. |

The published `Manatan-Server-Public` Android static library remains embedded as a local compatibility dependency. Fork-owned source/catalogue/job APIs replace only capabilities required by the manga-only product or unavailable in the published implementation.
