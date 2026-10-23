# Update signature test vectors

Made with minisign 0.12 (Alpine 3.22) in a throwaway container, with a key
generated for this purpose only; its secret half was deleted at once, so
these files sign nothing else (D170, `updates/tests.rs`):

```sh
minisign -G -p minisign.pub -s k.key
minisign -S -s k.key -m latest.json -t "shellpp 9.9.9 latest.json"
minisign -S -l -s k.key -m latest.json -x latest.json.legacy.minisig -t "shellpp 9.9.9 latest.json"
```

- `latest.json.minisig`: the prehashed signature (`ED`), as the release
  workflow makes it;
- `latest.json.legacy.minisig`: the legacy one (`Ed`), which the app refuses.
