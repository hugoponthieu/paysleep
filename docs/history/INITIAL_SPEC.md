# Paysleep spec

## Description

It helps with payslip management and storage. With one web app you can drag and drop one payslip and it will be stored att multiple places (drive, (proton, google), filesystem...)


## Web app

A single page with a form and a upload button that allow user to input company month and year.
It must show a readonly fs of all the payslip inside the browser.

## Backend definition

One back handles:

- the webapp exposure (use htmx, some nice components) that could be served with actix-web
- the work with the third parties (google, proton, filesystem, remote s3)

## Miscellanous

- Expose nice env variables for the management
- Make a rust workspace to work independently with each components 
- maybe we need a daemon that will run on the machine for filesystem
- No sql database, so you have to have a source somewhere
- will run inside kubernetes 
- nice log tracing and metrics

