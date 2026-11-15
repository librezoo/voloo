#!/bin/sh
# Wrap the artifact-style page.html into a standalone index.html for any static host.
set -e
cd "$(dirname "$0")/site"
{ printf '<!doctype html>\n<html lang="zh-CN">\n<head>\n<meta charset="utf-8">\n<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
  sed -n '1,/^<style>/{/^<style>/!p;}' page.html
  printf '</head>\n<body>\n<style>\n'
  sed -n '/^<style>/,${/^<style>/!p;}' page.html
  printf '\n</body>\n</html>\n'; } > index.html
echo "site/index.html written"
