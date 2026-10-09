#!/bin/sh
# A directory with more entries than anybody would list.
#
# The file panel's own acceptance is that fifty thousand entries open without drawing fifty thousand
# rows, and a test that has to make them itself tests the making: fifty thousand files over SFTP is
# thousands of round trips, where this is one second of `seq`, `awk` and `xargs`. The image runs
# everything in /custom-cont-init.d as root at start-up, so a lab that has been up at all has the
# fixture — and a container that is restarted does not make it twice.
set -e

# What the acceptance runs before this one left in /tmp. A directory that grows by a file every run
# makes the later cases depend on how many times the suite has been run, which is exactly the kind of
# accident a fixture exists to prevent.
rm -f /tmp/ss-e2e-*

big=/config/big-directory

if [ -d "$big" ] && [ "$(ls -A "$big" | wc -l)" -ge 50000 ]; then
  exit 0
fi

mkdir -p "$big"
cd "$big"
seq 1 50000 | awk '{ printf "entry-%06d.txt\n", $1 }' | xargs -n 200 touch

echo "the lab has 50000 entries in $big"
