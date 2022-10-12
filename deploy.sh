#!/usr/bin/env bash

# check vars
if [[ ! ${GITHUB_TOKEN} ]]; then echo "GITHUB_TOKEN is required" 1>&2; exit 1; fi
if [[ ! ${NAMESPACE} ]]; then echo "NAMESPACE is required" 1>&2; exit 1; fi

confirmation_needed=false
if [[ ${NAMESPACE} =~ ^p.* ]]; then confirmation_needed=true;fi


# check passed arguments
for argument in "$@"
do
  if [[ ${argument} == "-n" ]]
  then
    echo "Most probably arg -n has been used" 
    echo "Please, define namespace as env variable with name NAMESPACE" 1>&2
    exit 1
  fi
done

if [[ ${confirmation_needed} == true ]]
then
  echo "You are going to deploy to production namespace ${NAMESPACE}"
  read -p "Are you sure? (y/n) " -r
  echo
  if [[ ! ${REPLY} =~ ^[Yy].* ]]
  then
    exit 0
  fi
fi


# prepare env
rm -rf deploy
source deploy.init.sh


# run deploy
skaffold deploy "$@"
