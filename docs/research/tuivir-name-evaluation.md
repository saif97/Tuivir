# Tuivir name evaluation

_Checked 2026-08-12. Registry, domain, and trademark status can change._

**Decision:** adopted as the project name on 2026-08-12.

## Recommendation

**Yes: rename Virtui to Tuivir now, before the project is published, provided a small spoken-name test does not reveal persistent pronunciation or spelling trouble.**

This is not merely a preference between two sounds. `Virtui` already collides with multiple projects in the exact same product category and with an active registered US software mark. `Tuivir` had no exact collision on the identity and distribution surfaces checked. The local package is still version `0.1.0`, has `publish = false`, and the GitHub repository has no releases or stars, so the migration cost and installed-user disruption appear low.

Before announcing the new name, reserve the GitHub repository/organization or user identity, crate/package names, and the desired domain together. If the project will become commercial, ask a trademark lawyer to run a proper clearance search first.

## Collision and discoverability check

| Surface | `Tuivir` | `Virtui` | Assessment |
| --- | --- | --- | --- |
| GitHub repositories | GitHub's live name search returned **0** repositories containing `tuivir`. ([GitHub Search API](https://api.github.com/search/repositories?q=tuivir%20in:name&per_page=10)) | The same search returned **82** repositories containing `virtui`, including exact-name projects for KVM/libvirt VM management and an exact-name TUI automation project. ([GitHub Search API](https://api.github.com/search/repositories?q=virtui%20in:name&per_page=100), [uvewexyz/virtui](https://github.com/uvewexyz/virtui), [nixpig/virtui](https://github.com/nixpig/virtui), [theodric/virtui](https://github.com/theodric/virtui), [honeybadge-labs/virtui](https://github.com/honeybadge-labs/virtui)) | Strong advantage to Tuivir. Virtui's collisions are semantic, not incidental: several are terminal tools for virtual machines. |
| GitHub identity | An exact `tuivir` user/organization endpoint returned 404 when checked. ([GitHub Users API](https://api.github.com/users/tuivir)) | The exact `Virtui` GitHub user exists and owns an old `Virtui/Virtui` web UI for libvirt. ([GitHub user](https://github.com/Virtui), [repository](https://github.com/Virtui/Virtui)) | Tuivir offers a much cleaner identity, although a 404 is not a reservation guarantee. |
| crates.io | No exact crate and no search results. ([exact endpoint](https://crates.io/api/v1/crates/tuivir), [search](https://crates.io/api/v1/crates?q=tuivir&per_page=10)) | No exact crate and no search results. ([exact endpoint](https://crates.io/api/v1/crates/virtui), [search](https://crates.io/api/v1/crates?q=virtui&per_page=10)) | Tie today. crates.io names remain first-come, first-served, so reserve the chosen one promptly. |
| npm and PyPI | Exact package endpoints returned 404. ([npm](https://registry.npmjs.org/tuivir), [PyPI](https://pypi.org/pypi/tuivir/json)) | Exact package endpoints returned 404. ([npm](https://registry.npmjs.org/virtui), [PyPI](https://pypi.org/pypi/virtui/json)) | Tie. These are secondary surfaces for a Rust application but useful signals for ecosystem uniqueness. |
| AUR | No `tuivir`, `tuivir-bin`, or matching-name results. ([AUR RPC](https://aur.archlinux.org/rpc/v5/search/tuivir?by=name)) | No `virtui`, `virtui-bin`, or matching-name results. ([AUR RPC](https://aur.archlinux.org/rpc/v5/search/virtui?by=name)) | Tie, and currently low migration cost because the README says the intended AUR package has not yet been published. |
| Domains | The official `.com` registry returned no RDAP record for `tuivir.com`, and the `.dev` registry reported `tuivir.dev` not found. ([Verisign RDAP](https://rdap.verisign.com/com/v1/domain/tuivir.com), [Google Registry RDAP](https://pubapi.registry.google/rdap/domain/tuivir.dev)) | `virtui.com` has been registered since 2006; `virtui.dev` had no RDAP record. ([Verisign RDAP](https://rdap.verisign.com/com/v1/domain/virtui.com), [Google Registry RDAP](https://pubapi.registry.google/rdap/domain/virtui.dev)) | Tuivir has the better `.com` signal. A missing RDAP record suggests non-registration, not guaranteed purchasability or a right to use the name. |
| Existing software and marks | No exact software/product collision surfaced in the exact-name checks above. The accessible official trademark tools did not support a reproducible, linkable query result in this review. | Rice Lake currently refers to VIRTUi-family software in its product material, and the USPTO record for **VIRTUI**, serial 78438559 / registration 3143104, covers computer interface software in class 009 and is registered and renewed. ([Rice Lake product page](https://www.ricelake.com/products/iqube2-digital-diagnostic-junction-box/?print=true), [USPTO TSDR case](https://tsdr.uspto.gov/#caseNumber=78438559&caseSearchType=US_APPLICATION&caseType=DEFAULT&searchType=statusSearch)) | Strong advantage to Tuivir. The goods are not identical, but both are software interfaces, so Virtui carries avoidable legal and search ambiguity. |

## Brand fit: reasoned inference

These points are judgments, not registry facts:

- **Tuivir is more distinctive and more searchable.** A search for the coined word is more likely to lead to this project, whereas Virtui already describes or names several virtualization UIs.
- **It encodes the product neatly:** `TUI` + `vir` (terminal UI + virtualization). Putting `TUI` first distinguishes it from generic “virtual UI” names without tying it to Docker, Incus, or one Provider.
- **Its weak point is speech.** Readers could say “too-ee-veer,” “twee-veer,” or “too-ih-ver,” and someone who only hears it may not know the spelling. `Virtui` also has ambiguity (“virt-you-eye” versus “vir-twee”), so this is not a decisive loss, but it should be tested.
- **The spelling is compact but visually unusual.** Lowercase `tuivir` is memorable once seen, yet the `ui`/`vi` transition can invite transposition. A one-line pronunciation cue in the README may help during the launch period.

## Decision rule

Keep **Tuivir** if five or so likely users can hear the chosen pronunciation once and type `tuivir` with little prompting. If that test repeatedly fails, choose another distinctive coined name rather than returning to **Virtui**: the current name's same-category GitHub collisions and active software trademark are the larger long-term problem.

## Limits

This was a practical exact-name screen, not a comprehensive trademark clearance search. It did not exhaust confusingly similar spellings, unregistered/common-law uses, company registers in every country, every top-level domain, or every relevant trademark class and jurisdiction. WIPO itself recommends searching exact and similar marks and checking national/regional registers because its coverage can be incomplete. ([WIPO availability guidance](https://www.wipo.int/en/web/madrid-system/check-availability))

**This note is not legal advice.**
