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
      $group,
      $finished_callback,
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
     group => $group,
     finished_callback => $finished_callback,
    };

  return bless $self, $class;
}

sub group_is_done() {
  my ($self) = @_;

  foreach my $renderer (@{$self->{group}}) {
    return 0 unless $renderer->{is_done};
  }
  return $self->{is_done};
}

sub output_video_exists() {
  my ($self) = @_;

  return -e $self->full_output_video_filename();
}

sub full_output_video_filename() {
  my ($self) = @_;

  return "$self->{output_dirname}/$self->{output_video_filename}";
}

sub start($) {
  my ($self) = @_;

  return if($self->{is_running} || $self->{is_done});

  my $FFMPEG_OUT;

  open $FFMPEG_OUT, "$self->{ffmpeg_cmd} 2>&1 |" || die "cannot open FFMPEG: $!\n";
  $self->{FFMPEG_OUT} = \*$FFMPEG_OUT;
  $self->{frame_num} = 0;
  $self->{is_running} = 1;

  $self->{log}->timeLog($self->{output_video_filename}, "starting render of $self->{output_video_filename}", 10);
}

sub render_frame($) {
 my ($self) = @_;

# $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename}", 10);

 my $ret = undef;
 if(eof($self->{FFMPEG_OUT})) {
   $self->finish();
 } else {
   $/ = ""; # don't use newline for <> XXX explore 'local' for this more
   my $line = readline($self->{FFMPEG_OUT});
   $/ = "\n";
   if (defined $line) {
     #   $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} INSIDE", 10);

     # frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x 
     if ($line =~ /^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/) {
       $self->{frame_num} = $1;
       my $fps = $2;
#       $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} HIT $1 $2", 10);
       my $progress_percentage = $self->{frame_num} / $self->{raw_sequence_length};

       my $progress_bar = progress_bar(30, $progress_percentage); # XXX move to inc

       $progress_bar .= " rendering frame $self->{frame_num}/$self->{raw_sequence_length} ($fps fps) for";
       $self->{log}->log($self->{output_video_filename}, "$progress_bar $self->{output_video_filename}", 10, $progress_percentage);
     } else {
#       $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} MISS '$line'", 10);
     }
     $ret = 0;
   } else {
     $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} OUTSIDE", 10);
   }
 }
 return $ret;
}

sub finish($) {
  my ($self) = @_;

  # $self->{log}->timeLog($self->{output_video_filename}, "FINISH for $self->{output_video_filename}", 10);

  close $self->{FFMPEG_OUT};

  $self->{is_done} = 1;

  my $full_filename = $self->full_output_video_filename();

  if ($self->output_video_exists()) {
    my $video_size = sizeStringOf($full_filename);
    $self->{log}->timeLog($self->{output_video_filename}, "rendered $video_size $self->{frame_num} frame $full_filename", -1);
    $self->{result} = 1;
  } else {
    # failed, why?
    $self->{log}->timeLog($self->{output_video_filename}, "$full_filename render FAILED", -1);
    $self->{result} = 'error';	# XXX expose errors somehow
  }

  if ($self->group_is_done()) {
    # only after all renders are done for this group
    $self->{is_running} = 0;

    my $finished_callback = $self->{finished_callback};
    &$finished_callback($self);
  }
}


# a human readable string giving the size of a file (or total of a shell globbed list)
# XXX copied from backup.pl :(
# XXX fix this
sub sizeStringOf {
  my ($item) = @_;
  open DU, "du -ch $item |";
  $/ = "\n";
  while (<DU>) { return $1 if $_ =~ /^\s*([\d\w.]+)\s+total/; }
  close DU;
}

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
