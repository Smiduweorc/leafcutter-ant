# Thin dispatcher over scripts/tasks.sh, which is where the tasks are actually
# defined. Keeping the logic there means `just test` and the git hooks and CI
# all run the identical command.

_default:
    @scripts/tasks.sh help

# install the toolchain, dependencies and git hooks
setup:
    @scripts/tasks.sh setup

# format sources in place
fmt *files:
    @scripts/tasks.sh fmt {{ files }}

# static checks; fails on a problem, changes nothing
lint:
    @scripts/tasks.sh lint

# run the test suite
test:
    @scripts/tasks.sh test

# produce the build artifacts
build:
    @scripts/tasks.sh build

# regenerate CHANGELOG.md from the commit history
changelog:
    @scripts/tasks.sh changelog

# lint + test, as run before a release
preflight:
    @scripts/tasks.sh preflight

# (re)install the git hooks
hooks:
    @scripts/tasks.sh hooks

# remove build artifacts
clean:
    @scripts/tasks.sh clean

# cut a release: just release v1.2.3 (or patch | minor | major)
release version:
    @scripts/tasks.sh release {{ version }}

# list the tasks
help:
    @scripts/tasks.sh help
