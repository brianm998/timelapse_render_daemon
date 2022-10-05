package Renderer;

use strict;

# an object that can be used to render an image sequence into a video

sub new {
  my ($class,
      $log,
      $ffmpeg_cmd,
      $raw_sequence_length,
      $output_dirname,
      $output_video_filename,
      $image_sequence_name,
     )
      = @_;

  my $self =
    {
     log => $log,
     image_sequence_name => $image_sequence_name,
     ffmpeg_cmd => $ffmpeg_cmd,
     raw_sequence_length => $raw_sequence_length,
     output_dirname => $output_dirname,
     output_video_filename => $output_video_filename,
     frame_num =>  0,
     is_done => 0,
     is_running => 0,
    };

  return bless $self, $class;
}

sub start($) {
  my ($self) = @_;

  return if($self->{is_running} || $self->{is_done});

  my $FFMPEG_OUT;

  #print "FFMPEG: $self->{output_video_filename}\n";

  open FFMPEG_OUT, "$self->{ffmpeg_cmd} 2>&1 |" || die "cannot open FFMPEG: $!\n";
  $self->{FFMPEG_OUT} = \*FFMPEG_OUT;
  $self->{frame_num} = 0;
  $self->{is_running} = 1;

  $self->{log}->timeLog($self->{output_video_filename}, "starting render of $self->{output_video_filename}", 10);
}

sub render_frame($) {
 my ($self) = @_;

# $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename}", 10);

# print "self->{FFMPEG_OUT} $self->{FFMPEG_OUT}\n";
 my $ret = undef;
 $/ = ""; # don't use newline for <> XXX explore 'local' for this more
 if(eof($self->{FFMPEG_OUT})) {
   #print "EOF $self->{output_video_filename}\n";
   $self->finish();
 } else {
   my $line = readline($self->{FFMPEG_OUT});
   $/ = "\n";
   if (defined $line) {
     #   $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} INSIDE", 10);

     # frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x 
     if ($line =~ /^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/) {
       $self->{frame_num} = $1;
       my $fps = $2;
       #     $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} HIT $1 $2", 10);
       my $progress_percentage = $self->{frame_num} / $self->{raw_sequence_length};

       my $progress_bar = progress_bar(30, $progress_percentage); # XXX move to inc

       $progress_bar .= " rendering frame $self->{frame_num}/$self->{raw_sequence_length} ($fps fps) for";
       $self->{log}->log($self->{output_video_filename}, "$progress_bar $self->{output_video_filename}", 10);
     } else {
       #     $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} MISS '$line'", 10);
     }
     $ret = 0;
   } else {
     $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} OUTSIDE", 10);
#     $self->finish();
#     $ret = $self->{result};
   }
   $/ = "\n";
 }
 return $ret;
}

sub log_finished_result($) {
 my ($self) = @_;

# $self->{log}->timeLog($self->{output_video_filename}, "LOG_FINISHED_RESULT for $self->{output_video_filename}", 10);

 $self->{is_running} = 0;
 # XXX move this out
 if ($self->{result}) {
   my $sequence_size = sizeStringOf("$self->{output_dirname}/$self->{image_sequence_name}");
   # maybe delete the image sequence after successful render
   my $removed = 0;
   foreach my $delete_regex (@{$self->{config}{delete_sequence_after_render_regexes}}) {
     if ("$self->{output_dirname}/$self->{image_sequence_name}" =~ /$delete_regex/) {
       my $rm_cmd = "rm -rf $self->{output_dirname}/$self->{image_sequence_name}";
       $self->{log}->timeLog($self->{image_sequence_name}, "running $rm_cmd", 10);
       system($rm_cmd);
       $removed = 1;
       $self->{log}->timeLog($self->{image_sequence_name}, "done rendering videos from $sequence_size image sequence $self->{output_dirname}/$self->{image_sequence_name}, image sequence removed", -1);
     }
   }
   unless ($removed) {
     # potential bug where this doesn't show up when only
     # some videos needed to be rendered
     $self->{log}->timeLog($self->{image_sequence_name}, "done rendering videos from $sequence_size image sequence $self->{output_dirname}/$self->{image_sequence_name}", -1);
   }
 } else {
   $self->{log}->timeLog($self->{image_sequence_name}, "appears to have failed :(", -1);
 }
}

sub finish($) {
 my ($self) = @_;

 $self->{log}->timeLog($self->{output_video_filename}, "FINISH for $self->{output_video_filename}", 10);

 #print "self->{FFMPEG_OUT} $self->{FFMPEG_OUT}\n";
 my $result = close $self->{FFMPEG_OUT};
 #print "result '$result'\n";
 return unless(defined $result && length $result > 0);
 $self->{is_done} = 1;

 # XXX XXX XXX
 # XXX XXX XXX
 # XXX XXX XXX
# $result = 1;			# XXX the close above isn't always returning a value
 # XXX XXX XXX
 # XXX XXX XXX
 # XXX XXX XXX

 if ($result == 1) {
   my $video_size = sizeStringOf("$self->{output_dirname}/$self->{output_video_filename}");
   $self->{log}->timeLog($self->{output_video_filename}, "rendered $video_size $self->{frame_num} frame $self->{output_dirname}/$self->{output_video_filename}", -1);
   $self->{result} = 1;
 } else {
   # failed, why?
   $self->{log}->timeLog($self->{output_video_filename}, "$self->{output_dirname}/$self->{output_video_filename} render FAILED", -1);
   $self->{result} = 'error';	# XXX expose errors somehow
 }

 $self->log_finished_result();
}


# a human readable string giving the size of a file (or total of a shell globbed list)
# XXX copied from backup.pl :(
# XXX fix thisx
sub sizeStringOf {
  my ($item) = @_;
  open DU, "du -ch $item |";
  $/ = "\n";
  while (<DU>) { return $1 if $_ =~ /^\s*([\d\w.]+)\s+total/; }
  close DU;
}

sub sizeStringOf_WRONG {
  my ($item) = @_;
  return "9.4G      total\n";

}

# XXX copied, move this
sub progress_bar($$) {
  my ($length, $percentage) = @_;

  my $progress_bar = "[";
  for (my $i = 0 ; $i < $length ; $i++) {
    if ($i/$length < $percentage) {
      $progress_bar .= '*';
    } else {
      $progress_bar .= '-';
    }
  }
  $progress_bar .= "]";

  return $progress_bar;
}
1;
