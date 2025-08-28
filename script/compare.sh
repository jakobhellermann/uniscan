#!/bin/sh

cd $(dirname "$(readlink -f $0)")

ARG=${1:-GeoRock}

poop "./baseline.sh $ARG" "./new.sh $ARG"
