#!/bin/bash

sea-orm-cli migrate refresh -v && \
rm -rf fav_bili/src/entity/entity_inner/ && \
sea-orm-cli generate entity --expanded-format -o fav_bili/src/entity/entity_inner/
