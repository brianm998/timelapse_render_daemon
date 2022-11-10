
features:

automatically renders exported tiff/jpeg images sequences into a set of pre-defined video types, using ffmpeg
embeds exif file from source images into videos
supports a configurable number of concurrent renders
re-reads config file if it changes, and won't barf on invalid json, just uses existing config
can conditionally delete image sequences after render
shows progress of both image exports and video renders in realtime on console, w/ color
defaults to rendering full resolution video when no output resolution specified